# Open Transfer Manager

A light, open source transfer manager: download and upload, with resume and verification. The command is `otm`.

**Status: early prototype.** Nothing here is ready for everyday use yet. See [What has been tested](#what-has-been-tested) and [BENCHMARK.md](BENCHMARK.md).

## What works today

- **`otm get`:** downloads a file over several HTTP connections.
- **`otm relay`:** sends a file to S3-compatible storage or an SFTP server while it is still arriving. See [The relay](#the-relay).
- **`otm put`:** sends a file already on your computer, with the same resume and checks.
- **`otm queue`:** runs a list of transfers, a few at a time, and remembers what is finished. See [Queues](#queues).
- **SHA-256:** every transfer reports the digest of the whole file, and `--sha256 HEX` refuses a file that doesn't match.
- **`--json`:** one JSON object per event, for scripts. See [Scripts](#scripts).
- **One file on disk:** each piece goes straight to its place, and a journal lists what is safely written. A stopped download continues from there.
- **Pieces sized by time:** a request should last about 3 seconds at the measured speed. A new connection can take over the second half of the biggest piece left.
- **Automatic connection count:** it grows while total speed really grows, and falls when the server asks to slow down (`429`, `503`, `Retry-After`).
- **Checks on every answer:** `If-Range`, `ETag` and `Content-Range`. If the file changes on the server, the download starts over instead of mixing two versions.
- **Slow connections are replaced:** frozen or far slower connections are dropped, and the last pieces can be fetched twice.

## The relay

A part goes up as soon as it is complete on disk, in whatever order parts complete, so uploading starts long before the download ends.

- **Independent sides:** a slow destination lets the local file wait, and a slow source lets uploads wait.
- **Resume:** a journal records the parts the storage accepted. A stopped relay continues without sending them again, and without fetching a file that had already arrived.
- **No mixed versions:** if the source file changes, the upload is cancelled and starts over.
- **Nothing half-visible:** the object appears only at the very end, once the whole file arrived and every part was accepted.
- **Checks:** on S3, the storage checks each part against the SHA-256 in its signed request. On SFTP, which has no standard checksum, each part is read back and compared (turn this off with `--no-verify`). At the end, `otm` compares the stored size with the expected one. `--sha256` checks the whole local file just before the file appears at the destination. The stored copy itself is not read again as a whole.

### To S3-compatible storage

Keys come from `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY`.

```bash
otm relay "https://example.com/video.mp4" s3://my-bucket/videos/video.mp4 --endpoint https://s3.eu-west-1.amazonaws.com --region eu-west-1
```

### To an SFTP server

```bash
otm relay "https://example.com/video.mp4" sftp://me@example.org/backup/video.mp4
```

```bash
otm put video.mp4 sftp://me@example.org/backup/video.mp4
```

- **Server identity:** a server that isn't in `~/.ssh/known_hosts` is refused, and the message shows its fingerprint to compare. Then use `--host-key SHA256:...`, or `--accept-new-host-key` to remember it. A changed key is always refused.
- **Login:** a key file (`--identity`, by default `~/.ssh/id_ed25519`, with its passphrase in `OTM_SFTP_KEY_PASSPHRASE`) or a password in `OTM_SFTP_PASSWORD`. Secrets are never taken on the command line. `ssh-agent` isn't supported yet.
- **Existing files:** a file already at that name is kept, unless you pass `--overwrite`.
- **While it runs:** the data goes to `<name>.otmpart` and takes its real name only when complete.

## Queues

A queue file holds one JSON object per line. Blank lines and lines starting with `#` are skipped.

```json
{"op":"get","url":"https://example.com/a.zip","file":"a.zip"}
{"op":"relay","url":"https://example.com/b.zip","target":"s3://my-bucket/b.zip"}
{"op":"put","file":"c.zip","target":"sftp://me@example.org/c.zip","sha256":"..."}
```

```bash
otm queue transfers.jsonl --jobs 3
```

- **Failures stay contained:** one failed transfer doesn't stop the others.
- **Running it again:** finished transfers are recorded in `transfers.jsonl.state` and skipped, and failed ones are tried again. A local file that changed, or a download that was deleted, counts as new work. `--restart` forgets everything.
- **Safety:** a queue is refused if two transfers would use the same local file or the same target.
- **Options:** S3 and SFTP options apply to the whole queue.

## Scripts

With `--json`, standard output holds one JSON object per line:

```json
{"event":"started","id":0,"transfer":{"op":"get","source":"https://example.com/a.zip","target":"a.zip"}}
{"event":"progress","id":0,"size":1048576,"received":524288,"sent":0,"received_per_second":262144,"sent_per_second":0}
{"event":"done","id":0,"transfer":{"op":"get","source":"https://example.com/a.zip","target":"a.zip"},"result":{"size":1048576,"seconds":2.1,"sha256":"...","verified":"local","connections":2,"parts":0,"parts_resumed":0,"resumed_bytes":0,"retries":0,"restarts":0}}
```

- **Events:** `started`, `progress`, `done`, `failed` (with `error`), and for queues `skipped` and a final `summary`.
- **`verified`:** `local`, `parts-signed` (S3), `parts-read-back` (SFTP) or `parts-not-checked` (`--no-verify`).
- **Exit codes:** `0` all done, `1` a transfer failed, `2` wrong usage.
- **Secrets:** a password inside an address is removed from all output. Tokens in a URL's query, as in signed links, are not removed, so treat these logs with care.

## What has been tested

| Against | What it proves | What it doesn't |
|---|---|---|
| Simulated HTTP and S3 servers (`src/lab.rs`, `src/fake_s3.rs`) | Behaviour under cuts, freezes, errors and changing files | That a real S3 service agrees. No real service has answered yet; only the signature is checked, against the published AWS example |
| A simulated SFTP server, on real SSH (`src/fake_sftp.rs`) | Cuts, damaged data, server keys, logins | How real servers behave under those faults |
| OpenSSH's real `sftp-server` on a real disk (`tests/openssh_sftp.rs`) | The SFTP side: files, renames, resume and non-Latin names | OpenSSH's `sshd` and other servers: the SSH layer in that test isn't OpenSSH's |

Nothing has run over a real network yet. A GitHub Actions workflow runs the tests on Windows, Linux and macOS; so far they have only been run locally on Windows.

## Known limits

- **SFTP speed on distant servers:** pieces are written one small request at a time per connection, so round-trip time probably limits speed. This hasn't been measured over a real network.
- **All-or-nothing parts:** a part that fails is sent again from its start (8 MB by default).
- **Disk use:** the local file can reach the full file size when the destination is slow. A relay without `--keep-local` deletes it at the end.
- **S3:** only path-style addresses.
- **Not done yet:** the graphical interface, the system credential store, WebDAV, a speed limit and sharing the line between directions.

## Build and test

You need [Rust](https://www.rust-lang.org/) (edition 2021).

```bash
cargo test --release
```

The test against OpenSSH's `sftp-server` needs that program, which Git for Windows and most Linux OpenSSH packages include:

```bash
OTM_SFTP_SERVER=/path/to/sftp-server cargo test --release --test openssh_sftp -- --ignored --test-threads=1
```

```bash
cargo run --release --bin otm -- get "https://example.com/file.zip" file.zip
```

Running the same command again continues an interrupted download.

## Bench

```bash
cargo run --release --bin otm-lab -- bench --repeat 3
```

To compare with another engine, add `--old <program>`. It receives `LAB_URL` and `LAB_DIR` in its environment, must write the file to `LAB_DIR/out/lab.bin`, and must print a line `LAB_RESULT ok`.

## Layout

| Path | Role |
|---|---|
| `src/engine.rs` | The download engine |
| `src/relay.rs` | Receiving and sending at the same time |
| `src/destination.rs` | The interface every destination implements |
| `src/s3.rs` | The S3-compatible client |
| `src/sftp.rs` | The SFTP client |
| `src/fake_s3.rs`, `src/fake_sftp.rs` | Simulated servers for tests |
| `src/ranges.rs` | Sets of byte ranges |
| `src/lab.rs` | The test server and its scenarios |
| `src/bin/otm.rs` | The command line |
| `src/bin/otm-lab.rs` | The bench |
| `tests/scenarios.rs` | End-to-end download tests |
| `tests/relay.rs` | End-to-end relay tests to S3 |
| `tests/sftp.rs` | End-to-end tests over SFTP |
| `tests/openssh_sftp.rs` | The SFTP client against OpenSSH's `sftp-server` |

## Acceptable use

Transfer only what you have the right to transfer. This project does not bypass DRM, access controls or the limits a service sets.

## License

[MIT](LICENSE)

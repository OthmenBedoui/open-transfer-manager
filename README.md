# Open Transfer Manager

A light, open source transfer manager: download and upload, with resume and verification. The command is `otm`.

**Status: early prototype.** Nothing here is ready for everyday use yet. It has only run against simulated local servers, never against real storage. See [BENCHMARK.md](BENCHMARK.md).

## What works today

- **`otm get`:** downloads a file over several HTTP connections.
- **`otm relay`:** sends a file to S3-compatible storage while it is still arriving. See [The relay](#the-relay).
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
- **Checks:** the storage checks each part against the SHA-256 in its signed request. At the end, `otm` compares the stored size with the expected one. There is no whole-file checksum comparison yet.

```bash
export AWS_ACCESS_KEY_ID=...
```

```bash
export AWS_SECRET_ACCESS_KEY=...
```

```bash
otm relay "https://example.com/video.mp4" s3://my-bucket/videos/video.mp4 --endpoint https://s3.eu-west-1.amazonaws.com --region eu-west-1
```

**Limits today:** only S3-compatible destinations with path-style addresses; SFTP is not written yet. The tests use a simulated storage server, not a real one: the signature matches the published AWS example, but no real service has answered it. The local file can grow to the full file size when the destination is slow, as there is no limit on it yet.

## Build and test

You need [Rust](https://www.rust-lang.org/) (edition 2021).

```bash
cargo test --release
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
| `src/s3.rs` | The S3-compatible client |
| `src/fake_s3.rs` | A simulated storage server for tests |
| `src/ranges.rs` | Sets of byte ranges |
| `src/lab.rs` | The test server and its scenarios |
| `src/bin/otm.rs` | The command line |
| `src/bin/otm-lab.rs` | The bench |
| `tests/scenarios.rs` | End-to-end download tests |
| `tests/relay.rs` | End-to-end relay tests |

## Acceptable use

Transfer only what you have the right to transfer. This project does not bypass DRM, access controls or the limits a service sets.

## License

[MIT](LICENSE)

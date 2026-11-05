# Open Transfer Manager

A light, open source transfer manager: download and upload, with resume and verification. The command is `otm`.

**Status: early prototype.** Nothing here is ready for everyday use yet. It has only run against simulated local servers. See [BENCHMARK.md](BENCHMARK.md).

## What works today

- **`otm get`:** downloads a file over several HTTP connections.
- **One file on disk:** each piece goes straight to its place, and a journal lists what is safely written. A stopped download continues from there.
- **Pieces sized by time:** a request should last about 3 seconds at the measured speed. A new connection can take over the second half of the biggest piece left.
- **Automatic connection count:** it grows while total speed really grows, and falls when the server asks to slow down (`429`, `503`, `Retry-After`).
- **Checks on every answer:** `If-Range`, `ETag` and `Content-Range`. If the file changes on the server, the download starts over instead of mixing two versions.
- **Slow connections are replaced:** frozen or far slower connections are dropped, and the last pieces can be fetched twice.

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
| `src/ranges.rs` | Sets of byte ranges |
| `src/lab.rs` | The test server and its scenarios |
| `src/bin/otm.rs` | The command line |
| `src/bin/otm-lab.rs` | The bench |
| `tests/scenarios.rs` | End-to-end tests |

## Acceptable use

Transfer only what you have the right to transfer. This project does not bypass DRM, access controls or the limits a service sets.

## License

[MIT](LICENSE)

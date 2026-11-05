# Changelog

## 0.1.0 (unreleased)

First prototype, for people comfortable with a command line. Not yet tested on real networks.

- `otm get`: multi-connection downloads with resume and change detection.
- `otm relay`: send to S3-compatible storage or SFTP while the file arrives.
- `otm put`: send a local file with the same resume and checks.
- `otm queue`: run a list of transfers, a few at a time, skipping finished ones on the next run.
- SHA-256 of every transferred file, and `--sha256` to require a value.
- `--json` output and stable exit codes for scripts.
- SFTP server key checking, and read-back of each piece.

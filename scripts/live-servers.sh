#!/usr/bin/env bash
# Runs a command while local SFTP, FTP, WebDAV and S3 test servers are up (rclone serve).
# Used by CI for real uploads and transfers:  scripts/live-servers.sh RCLONE ROOT -- CMD...
# The servers listen on 127.0.0.1 only and serve ROOT/{sftp,ftp,dav,s3}.
set -euo pipefail

R="${1:?rclone path}"; ROOT="${2:?served folder}"; shift 2
[ "${1:-}" = "--" ] && shift
mkdir -p "$ROOT/sftp" "$ROOT/ftp" "$ROOT/dav" "$ROOT/s3/bucket1"
pids=()
"$R" serve sftp "$ROOT/sftp" --addr 127.0.0.1:2022 --user tester --pass 's3cret pw' --config "" & pids+=($!)
"$R" serve ftp "$ROOT/ftp" --addr 127.0.0.1:2121 --user tester --pass 's3cret pw' --config "" & pids+=($!)
"$R" serve webdav "$ROOT/dav" --addr 127.0.0.1:8081 --user tester --pass 's3cret pw' --config "" & pids+=($!)
"$R" serve s3 "$ROOT/s3" --addr 127.0.0.1:8082 --auth-key AKTEST,SKTEST123 --config "" & pids+=($!)
trap 'kill "${pids[@]}" 2>/dev/null || true' EXIT
sleep 5
OTM_LIVE_DIR="$ROOT" "$@"

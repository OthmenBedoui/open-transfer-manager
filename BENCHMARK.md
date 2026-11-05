# Benchmark

This compares **otm** with **baseline**, an earlier unpublished multi-connection engine set to 8 connections. The baseline program is not part of this repository, so only the `otm` rows can be reproduced from it.

## Summary

- **Reliability:** when the file changed mid-transfer, baseline produced a corrupt file on every run while reporting success. otm was correct on every run.
- **Frozen connections:** otm took 6.8 s against 35.3 s.
- **Cut connections:** otm took 3.4 s against 5.4 s.
- **A server asking to slow down:** otm received 3 refusals against 16, but was slower (6.1 s against 4.9 s).
- **Raw speed:** otm is still slower with a per-connection limit (4.3 s against 3.7 s) and with slow connections (6.5 s against 5.5 s).

## Results

Median of 3 runs per engine and scenario.

| Scenario | Engine | Correct | Time (s) | Connections | Requests | Sent (MB) | 429 received | Peak disk (MB) |
|---|---|---|---|---|---|---|---|---|
| fast | baseline | 3/3 | 1.3 | 3 | 1 | 301 | 0 | 347 |
| fast | otm | 3/3 | 1.5 | 4 | 11 | 301 | 0 | 300 |
| per-connection-limit | baseline | 3/3 | 3.7 | 9 | 7 | 64 | 0 | 64 |
| per-connection-limit | otm | 3/3 | 4.3 | 11 | 19 | 65 | 0 | 64 |
| saturated-line | baseline | 3/3 | 4.5 | 23 | 21 | 65 | 0 | 64 |
| saturated-line | otm | 3/3 | 4.3 | 4 | 7 | 64 | 0 | 64 |
| slow-connections | baseline | 3/3 | 5.5 | 34 | 32 | 65 | 0 | 64 |
| slow-connections | otm | 3/3 | 6.5 | 6 | 13 | 64 | 0 | 64 |
| stalled-connections | baseline | 3/3 | 35.3 | 62 | 60 | 66 | 0 | 64 |
| stalled-connections | otm | 3/3 | 6.8 | 6 | 14 | 64 | 0 | 64 |
| server-429 | baseline | 3/3 | 4.9 | 26 | 24 | 64 | 16 | 64 |
| server-429 | otm | 3/3 | 6.1 | 7 | 18 | 64 | 3 | 64 |
| cuts | baseline | 3/3 | 5.4 | 53 | 51 | 66 | 0 | 64 |
| cuts | otm | 3/3 | 3.4 | 19 | 25 | 64 | 0 | 64 |
| changed-file | baseline | **0/3** | 3.7 | 9 | 7 | 64 | 0 | 64 |
| changed-file | otm | 3/3 | 6.8 | 15 | 27 | 85 | 0 | 64 |
| no-range | baseline | 3/3 | 4.1 | 2 | 0 | 32 | 0 | 32 |
| no-range | otm | 3/3 | 4.0 | 2 | 0 | 32 | 0 | 30 |

These runs predate the rename: scenario and engine labels were translated, and the engine's logic has not changed since.

## Reading the results

- **changed-file matters most.** otm is slower because it starts over. baseline is faster because it keeps a wrong file.
- **Small gaps mean nothing.** In `fast`, baseline varied from 1.0 to 2.2 s between runs. The gaps in `saturated-line` and `no-range` are also too small to conclude.
- **A careful start.** otm begins with 2 connections and reconsiders every second, probably a large part of its delay in `per-connection-limit`. A faster climb would likely draw more refusals from servers; this trade-off is not settled.
- **Disk use.** baseline peaked at about 1.16 times the file size, sampled every 100 ms, so a brief peak could be missed. otm takes the final size from the start.

## Method

- Each run starts a server and an engine, each in its own process, both built in release mode.
- **Correct:** the file equals one whole version of the server's file, byte for byte. A mix of two versions is an error.
- **Connections:** TCP connections the server accepted, including the bench's own when it reads the counters at the end.
- **Peak disk:** total size of the working folder, sampled every 100 ms.
- **Machine:** one Windows 11 PC.

## Limits

- **Local only:** everything goes through `127.0.0.1`, with no network latency or packet loss.
- **Simulated server:** no real CDN was tested, and real servers vary more.
- **HTTP/1.1 only:** no HTTP/2 or HTTP/3, and TLS was not measured.
- **One size per scenario:** no very small or multi-gigabyte files.
- **Few runs:** 3 per case on one machine. Gaps of a few tenths of a second are not significant.
- **Chosen scenarios:** they expose weaknesses and do not represent average use. They were also used to tune otm, which may favour it.
- **Limited change detection:** with no `ETag` or `Last-Modified`, a change at equal size stays invisible.

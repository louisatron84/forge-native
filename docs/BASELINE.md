# Performance baseline

The numbers the native app is measured against (budgets: `docs/PLAN.md`).

## Electron Forge 0.50.0

| Measure | Value | How |
|---|---|---|
| Idle memory, one window, small project | ~530 MB | Linux tester round (AppImage 0.46.1), all Forge processes |
| Peak memory, small session | ~630 MB | same |
| Several windows open (owner's machine, Windows) | ~940 MB | sum of all `Forge.exe` working sets, 2026-11-08 |
| Installer size | 133 MB (Setup), 117 MB (portable) | release 0.50.0 |

A clean single-window measurement on Windows is still to do: the locally
built `release\win-unpacked` copy exits at once, so it needs the installed
0.50.0 (or `npm run dev`).

## Native skeleton (M0)

| Measure | Value | How |
|---|---|---|
| Empty window (`gpui-kit` 0.9.0, debug build) | 109 MB working set, 98 MB private | Windows 11, 2026-11-08, 6 s after start |
| First build of the GPUI stack | 2 min 52 s | `cargo build -p forge-app`, 16 cores |

Release builds and the editor/Graph stress cases are measured as they land.

## Reproducible model measurements (2026-11-14)

Run `python scripts/perf.py` on each native machine; output is
`target/perf-report.json`. CI uploads this report for Windows and Linux.

| Measure | Windows x64 optimized build | Scope |
| --- | --- | --- |
| Edit, tokenize and remove one character in a warm 100k-line buffer, p95 | 0.0025 ms | 1,000 model iterations; excludes screen/render latency |
| Layout of a 400-node, 399-edge graph, p95 | 0.3486 ms | 100 iterations; excludes GPU drawing |
| Standalone NSIS installer | 10,316,562 bytes | Under the 40 MB installer budget |
| Update-enabled Velopack installer | 27,260,015 bytes | Includes update/runtime setup; under 40 MB |

Toolchain: rustc 1.99.1, Windows build 26300, AMD64. These timings do **not**
establish the desktop budgets: idle memory, editable-window cold start,
keystroke-to-screen latency, scrolling FPS and graph layout plus drawing still
need instrumented desktop measurements on the reference Windows/Linux machines.
SHIP-09 stays open until those measurements are recorded and meet the targets.

## Incremental highlighting investigation (2026-11-14)

The user supplied an external bot's table reporting missed typing, scrolling,
graph and full-process memory budgets. Its hardware, OS, workload and timing
method were not supplied, so those figures are not a reproducible local baseline.

An expanded `perf_text` example reproduces unnecessary work omitted by the
original single-line measurement: warm a 100k-line Python file, insert and delete
a character on its first line, and request the final line's tokens after each
edit. Thirty release-build iterations on this Windows machine:

| CPU workload (p95) | Before | After |
| --- | --- | --- |
| Edit/delete plus later-file syntax state propagation | 116.0426 ms | 0.0043 ms |
| Existing warm middle-line model benchmark | 0.0024 ms | 0.0013 ms |

Run `cargo run --release --offline -p forge-text --example perf_text -j 1`.
The highlighter now reuses its old state tail once lexical state converges beyond
all pending edits. Line insertions/deletions discard the misaligned tail.
This also avoids repeated minimap warm-up frames after ordinary typing. Tests
compare incremental results against a fresh highlighter across batched edits,
Python multiline strings and JSONC comments.

These are CPU model timings, **not** input-to-screen latency or scrolling FPS.
Graph GPU draw time, whole-session idle memory and the supplied desktop latency
figures remain unverified; no performance budget is marked complete by this change.

## Output backpressure investigation (2026-11-17)

The existing release benchmark pushes 200,000 alternating stdout/stderr lines
through a 200,000-byte queue and drains the retained output. Removing the first
element of a vector repeatedly shifted all remaining pieces during trimming.
Using a deque keeps those removals constant time and preserves stream order.

| CPU workload | Before | After |
| --- | --- | --- |
| Alternating-stream queue, push/trim/drain | 24,223.162 ms | 11.787 ms |

Run `cargo test -p forge-core --release -j 1 alternating_stream_trim_benchmark -- --ignored --nocapture`.
These are single-run Windows x64 optimized measurements. A separate regression
test verifies that trimming at a byte inside a UTF-8 character retains subsequent
complete lines instead of discarding the whole piece. This measurement does not
establish any of the remaining desktop performance budgets.

# Forge native: rewriting Forge in Rust (GPUI)

## Context
Forge (Electron + TypeScript, v0.50.0; about 32k lines of TS, 2k CSS, 2k Python helpers, 9k lines of tests) isn't slow today. It idles at about 0.53 GB, measured in the tester round. The user wants to **future-proof** it: as features pile up, a native app should keep Forge fast and light. Discord's Nov 2026 move from Electron to Rust/GPUI (an early build showed ~290 MB against 700 MB+) prompted this. The user prefers **quality over quantity**: no new features while the rewrite happens, in return for a large jump in speed. Since Forge grew ~47k lines in November alone, rewriting sooner is cheaper than later.

**Decided by the user:**
- **Toolkit:** fully native Rust with **GPUI** (the Zed editor's toolkit, also used by Discord's rebuild). Not Tauri.
- **The Electron app:** **frozen completely** from now on. 0.50.0 is its last release until the native app replaces it.
- **Repo:** a **new, private repo** with a fitting name and a **more locked-down license**.
- **Plan first:** plan the whole change before building anything.

**My calls (open to change):**
- **Repo name:** `forge-native` (private).
- **License:** proprietary, "All rights reserved". Others may not copy, modify or redistribute the code; the user can relicense later. The Electron repo stays public and MIT; copies already published stay MIT, and that's unaffected.
- **Product name:** stays **Forge**, with version **1.0.0** for the first native release that replaces Electron.

## Performance budgets (the reason for the rewrite)
Each budget is measured on the same machine against Electron Forge 0.50.0, recorded in M0.

| Budget | Target | Notes |
|---|---|---|
| Idle memory (Forge's own processes) | ≤ 150 MB | The language server and Python are counted separately |
| Cold start to an editable window | ≤ 0.6 s | |
| Keystroke to screen, 100k-line file | ≤ 8 ms | |
| Scrolling | 60+ fps | |
| Graph of 400 nodes | ≤ 100 ms to lay out and draw | |
| Installer size | ≤ 40 MB | Electron's is ~133 MB |

The Python language server (basedpyright, a Node program) stays: it's the best Python checker, and rewriting it isn't the goal. Its memory is reported separately. The native app starts it lazily (only once a Python file is opened) and caps it. Swapping to Astral's `ty` is a later option, behind a setting, once it's mature.

## Architecture (new repo `forge-native`)
A Cargo workspace. Each crate does one job, and only `forge-app` knows about GPUI:

| Crate | What it holds | Ported from (Electron repo) |
|---|---|---|
| `forge-core` | Project model and pure logic: project graph, events, database schema, profile, memory log, network log, layouts, run configs, templates, settings, release notes, suggestions | `src/shared/*.ts` (6.3k lines, all pure: almost a line-for-line port) |
| `forge-text` | Text buffer (rope, `ropey`), cursors and multi-cursor, selection, undo, folding, indentation, find, Python tokenizer and highlighting | `src/renderer/editor/*`, `src/shared/` tokenizer and folding |
| `forge-lsp` | Language-server client (JSON-RPC framing, basedpyright config incl. the `ANALYSIS` overrides, semantic tokens) | `src/main/jsonrpc.ts`, `src/renderer/lsp/*` |
| `forge-dap` | Debug-adapter client for debugpy: sessions, child processes, breakpoints, the debug trail | `src/main/debugAdapter.ts`, `src/renderer/debug/*` |
| `forge-run` | Running Python: interpreter discovery, `.env`, the traced-run socket server (token, message parsing) | `src/main/python.ts`, `runTrace.ts`, `dotenvFile.ts` |
| `forge-term` | Terminal: `alacritty_terminal` for the screen model, `portable-pty` for shells | xterm.js plus node-pty today |
| `forge-scan` | Runs `projectScan.py` / `dataflow.py` with overrides and `--only` | `src/main/projectScan.ts` |
| `forge-update` | Velopack updates: background download, install on close, What's new | `src/main/updates.ts` |
| `forge-app` | GPUI shell: windows, docking and tasks, every panel and view, themes (Molten and Plain), keymap, menus | `src/renderer/*` (21.9k lines: the real rewrite) |
| `python/` | `runTrace.py`, `projectScan.py`, `dataflow.py`, blocks: **copied unchanged**, embedded with `include_str!` | `src/main/*.py` |

**Kept as-is:** the Python helpers and their message formats (`tick`, `prof`, `mem`, `net`, `rec`, `io`, `data`). They're language-neutral, so the whole tracer and analysis layer carries over.

**Tests:** the 9k lines of vitest are the spec. Every pure suite in `src/shared/*.test.ts` and `src/renderer/editor/*.test.ts` is ported to Rust `#[test]`s with the same cases. The real-Python suites (`runTrace.test.ts`, `projectScan.test.ts`, `dbScan.test.ts`, `eventScan.test.ts`, `basedpyright.test.ts`) are ported as integration tests.

## Milestones
Each milestone ends with a runnable app, a perf check against the budgets, and a commit on a branch, merged by PR.

**M0. Foundations and the go/no-go gate** (first sessions)
- Create the private repo `forge-native`: proprietary LICENSE, README, CI (fmt, clippy, tests on Windows and Linux), branch protection like IDEforfun.
- Measure Electron Forge 0.50.0: per-process memory, start time, typing latency. Record the numbers in `docs/BASELINE.md`.
- Settle the unknowns found in research:
  - **Which GPUI to depend on:** upstream crates.io `gpui` 0.2.x, a pinned Zed git revision, or a maintained standalone fork. Choose for working Windows and Linux (X11 and Wayland) support.
  - **Velopack:** confirm it can apply an update when the app exits (`wait_exit_then_apply_updates`) on Windows and Linux.
- Build a skeleton: window, folder tree, the hand-written editor rendering a 100k-line file with highlighting, and 400 Graph boxes. Measure it.
- **Gate:** if the skeleton misses the budgets, or GPUI can't handle IME or text input on Windows and Linux, stop. Come back with the numbers and a recommendation (another toolkit, or stay on Electron) before going further.

**M1. Editor**, mirroring CHECKLIST Tier 1
Buffer, cursors, multi-cursor, selection, clipboard, undo/redo, auto-indent, folding, highlighting, find/replace, big-file scrolling, tabs, split view, dirty markers and saving.

**M2. IDE foundations**, mirroring Tier 2
Run Python and Output, terminal, command palette and keymap, settings (same JSON format), Molten and Plain themes with bundled fonts, docking with the four tasks (Code, Connect, Debug, Run) and their layouts, the Welcome page, recent folders.

**M3. Language and tooling**, mirroring Tier 3
- basedpyright: diagnostics, completion, hover, go to definition, references, rename, outline, semantic highlighting, quick fixes.
- Ruff and Black formatting, the Problems panel.
- Project search, Git (status, diff, commit, history), the Tests panel, the debugger.

**M4. Visual Forge**, mirroring Tiers 4 and 5 and the feature ideas
- Project Graph (port `graph/layout.ts` and `viewModel.ts` to `forge-core`, draw on a GPUI canvas): Modules, Calls and Events views.
- Live runs: sparks, replay, the profiler's times and heat.
- Network, Pipeline / data flow, API boxes and OpenAPI, the Toolbox, debugger cards and object graph, recursion trees.
- Project Map, Database, Memory, Where you are, In this file, suggestions, templates.

**M5. Ship 1.0**
- Velopack installers for Windows and Linux (AppImage, plus .deb if Velopack supports it, else a separate packager).
- Updates: background download, install on close, What's new on Welcome.
- Import Electron Forge's settings, keybindings, recent folders and saved templates from its `userData` folder on first run.
- A tester round (same brief as before), then release 1.0.0.

**Moving Electron users over:** unfreeze the Electron repo once for **0.51.0**. Its release notes point to Forge 1.0. Electron's updater then installs that final release, whose Welcome page carries a "Forge 1.0 is here: download" card.
- **Release locations:** the private repo's releases aren't public, so native releases are published from a **public releases-only repo** (e.g. `forge-releases`, binaries and notes only), or another host. Velopack's update feed points there.
- **Website:** forgecode.uk switches its downloads to 1.0.

## Risks and how they're handled
| Risk | Handling |
|---|---|
| GPUI is pre-1.0 and its API changes | Pin one version or revision; upgrade on purpose, one PR at a time |
| GPUI's Windows and Linux maturity | M0 gate tests both before anything else is built |
| Text input: IME, dead keys, accessibility | Tested in M0. Screen-reader support is weaker than the browser's; recorded as a known gap |
| The terminal | Use `alacritty_terminal`, the crate Zed's terminal is built on |
| Months with no Electron releases | The user chose this; testers stay on 0.50.0 |
| Scope creep during the port | No new features until 1.0; ideas go in CHECKLIST "after 1.0" |

## Working rules carried over
- Unit and integration tests and `cargo clippy` are fine. In-app or end-to-end testing only when the user asks.
- No subagents.
- PRs into `main`, merged by the user; releases published by the user.
- No email in public files.
- Write the TDD test from the existing vitest case first, then port the code.

## Verification
- **Each milestone:**
  - `cargo test --workspace` (ported suites plus the real-Python integration tests) and `cargo clippy -- -D warnings`.
  - A perf script (`scripts/perf`) that records memory, start time, typing latency and Graph draw time, compared with `docs/BASELINE.md` and the budgets.
- **Before 1.0:** a full tester round on Windows and Linux, the same brief as the 0.46.1 round, and a test of the update path:
  1. install 1.0.0;
  2. publish 1.0.1;
  3. close Forge;
  4. check it reopens on 1.0.1 with What's new showing.
- **The rewrite's success criterion:** 1.0 does everything 0.50.0 does (checklist parity) and meets the budgets.

## Rough size
M0: 2–3 sessions. M1–M2: the biggest chunk. M3–M4: the most features. M5: 2–3 sessions. Realistically **months** at hobby pace. Each milestone is usable on its own, so progress shows along the way.

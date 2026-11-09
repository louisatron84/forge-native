# Forge native progress ledger

Every feature Electron Forge 0.50.0 shipped, taken from its `docs/CHECKLIST.md` ([x] items), plus the shipping work.
Tick an item only when its Rust version exists and its ported tests pass. Details of each item: the old checklist line
with the same text, and the old source and tests. This ledger is how work resumes after a break.

## M1: Editor (Tier 1) (21)

- [x] T1-01 Open a folder, show the file tree, expand/collapse folders
- [x] T1-02 Create, rename and delete files and folders from the tree
- [x] T1-03 Open a file and render its text with line numbers in a monospace font
- [x] T1-04 Text buffer model: insert, delete, line/column ↔ offset
- [x] T1-05 Cursor movement: arrows, Home/End, PgUp/PgDn, click to place
- [x] T1-06 Typing, Backspace/Delete, Enter (with Python auto-indent after `:`)
- [x] T1-07 Selection (shift+arrows, mouse drag, double/triple-click, Ctrl+A), copy/cut/paste, Tab/Shift+Tab indent
- [x] T1-08 Save (Ctrl+S), dirty marker, warning about unsaved changes on close
- [x] T1-09 Undo/redo
- [x] T1-10 Scrolling that only renders visible lines (50k lines opens in ~80ms, v0.5)
- [x] T1-11 Multi-cursor editing
- [x] T1-12 Bracket matching and automatic bracket completion
- [x] T1-13 Automatic quote completion
- [x] T1-14 Comment/uncomment selection
- [x] T1-15 Code folding (functions, classes and blocks)
- [x] T1-16 Minimap
- [x] T1-17 Multiple editor groups / split view
- [x] T1-18 File encoding detection and conversion
- [x] T1-19 CRLF/LF line endings: shown in the status bar, click to convert
- [x] T1-20 Large-file mode and graceful handling of binary files
- [x] T1-21 Recovery of unsaved buffers after unexpected crashes

## M2: IDE foundations (Tier 2) (19)

- [x] T2-01 Tabs for several open files
- [x] T2-02 Python syntax highlighting (own tokenizer, v0.5)
- [x] T2-03 Find/replace in the file (case, whole word, regex with $1 groups, v0.7)
- [x] T2-04 Run the current Python file and show its output in a panel (F5, auto-save, venv detection, `input()`, clickable tracebacks, v0.8)
- [x] T2-05 Integrated terminal (xterm.js + node-pty, Ctrl+`, v0.9)
- [x] T2-06 Command palette (Ctrl+Shift+P), quick open (Ctrl+P), go to line (Ctrl+G)
- [x] T2-07 Settings (font size, theme, tab width, Python interpreter, Ctrl+= / Ctrl+-)
- [x] T2-08 Multiple terminal sessions
- [x] T2-09 Terminal profiles and shell selection
- [x] T2-10 Configurable keyboard shortcuts
- [x] T2-11 Recent projects
- [x] T2-12 Workspace persistence (restore tabs, terminal sessions and layout)
- [x] T2-13 Project-specific settings
- [x] T2-14 Task runner for predefined project commands
- [x] T2-15 Run configurations (arguments, environment variables and working directory)
- [x] T2-16 Python environment management (create, select and manage virtual environments)
- [x] T2-17 Dependency management integration (pip, requirements.txt, pyproject.toml)
- [x] T2-18 Updates
- [x] T2-19 Test runner integration (pytest, unittest)

## M3: Language and tooling (Tier 3) (27)

- [x] T3-01 Language Server Protocol client (Pyright)
- [x] T3-02 Search across the project (Ctrl+Shift+F, case / word / regex, v0.12)
- [x] T3-03 Git status in the file tree (M/U/A/D badges, folder dots, auto-refresh, v0.12)
- [x] T3-04 Debugger (DAP, debugpy): breakpoints, stepping, call stack, variables, debug console with expression evaluation, hover values, pause on uncaught exceptions
- [x] T3-05 Find references
- [x] T3-06 Rename symbol across project
- [x] T3-07 Go to implementation
- [x] T3-08 Call hierarchy
- [x] T3-09 Document symbols / outline panel
- [x] T3-10 Import organisation and automatic import suggestions
- [x] T3-11 Code actions and quick fixes
- [x] T3-12 Format document / selection (Black, Ruff)
- [x] T3-13 Linting integration (Ruff)
- [x] T3-14 Semantic syntax highlighting (Pyright doesn't provide semantic tokens; needs basedpyright or our own analysis)
- [x] T3-15 Language-support architecture: language integrations instead of Python-specific editor logic, so several language servers can run side by side (groundwork for Tier 6.2)
- [x] T3-16 Git diff viewer
- [x] T3-17 Stage/unstage individual hunks
- [x] T3-18 Commit interface
- [x] T3-19 Branch switching and creation
- [x] T3-20 Merge conflict resolution
- [x] T3-21 Git history and file blame, with a graph view
- [x] T3-22 Support `input()` while debugging (run the debugged program in a terminal)
- [x] T3-23 Conditional breakpoints
- [x] T3-24 Watch expressions
- [x] T3-25 Option to pause on every raised exception, not just uncaught ones
- [x] T3-26 Breakpoint management panel (and keep breakpoints between sessions)
- [x] T3-27 Debug multiple processes

## M4: Visual Forge (Tiers 4-5, ideas, templates) (87)

- [ ] T4-01 Design a distinctive workspace layout
- [ ] T4-02 Rethink how files, terminals, graphs and debugging coexist
- [ ] T4-03 Customisable workspace panels
- [ ] T4-04 Dockable panels and floating windows
- [ ] T4-05 Multiple workspace layouts (coding, debugging and visualisation)
- [ ] T4-06 Context-aware interface that surfaces relevant tools for the current task
- [ ] T4-07 Customisable information density
- [ ] T4-08 Animated transitions between workspace states
- [ ] T4-09 Visual navigation through projects
- [ ] T4-10 Consistent visual language for graphs, code, debugging and terminal
- [ ] T4-11 Forge task workspace: Code / Connect / Debug / Run, movable panels, Edit Layout mode (Part 2 of docs/superpowers/specs/2026-11-06-forge-design.md)
- [ ] T4-12 Live run feedback on the graph
- [ ] T4-13 Connect task and the API box (Part 3 of the same spec; also covers parts of 5.3)
- [ ] T5-01 Parse the project using Python's `ast` (a helper script run with the project's own Python) (5.1 Project intelligence graph)
- [ ] T5-02 Build a project dependency graph: files, modules, classes, functions, imports (5.1 Project intelligence graph)
- [ ] T5-03 Function-to-function call relationships (5.1 Project intelligence graph)
- [ ] T5-04 Detect API endpoints called (`requests`, `httpx`, `aiohttp`) (5.1 Project intelligence graph)
- [ ] T5-05 Detect API routes (FastAPI, Flask) (5.1 Project intelligence graph)
- [ ] T5-06 Detect environment variables (os.environ[...], os.getenv, environ.get) (5.1 Project intelligence graph)
- [ ] T5-07 Display dependencies between components (5.1 Project intelligence graph)
- [ ] T5-08 Click a graph node to navigate to its source code (double-click; details on single click) (5.1 Project intelligence graph)
- [ ] T5-09 Highlight a node's dependencies and dependants (hover or select: uses in blue, used by in orange) (5.1 Project intelligence graph)
- [ ] T5-10 Filter graph by type, file or relationship (5.1 Project intelligence graph)
- [ ] T5-11 Detect circular imports (red edges; listed in the findings) (5.1 Project intelligence graph)
- [ ] T5-12 Identify unused functions and potentially unreachable code where statically determinable (5.1 Project intelligence graph)
- [ ] T5-13 Runtime instrumentation for `requests`, `httpx` and `aiohttp` (5.2 Live network visualisation)
- [ ] T5-14 Capture socket-level connections where appropriate (5.2 Live network visualisation)
- [ ] T5-15 Stream runtime events to the IDE (5.2 Live network visualisation)
- [ ] T5-16 Animated network graph (5.2 Live network visualisation)
- [ ] T5-17 Display HTTP method, URL, status and response time (5.2 Live network visualisation)
- [ ] T5-18 Click a connection to inspect request/response details (5.2 Live network visualisation)
- [ ] T5-19 Request history and filtering (5.2 Live network visualisation)
- [ ] T5-20 Group requests by host and originating function (5.2 Live network visualisation)
- [ ] T5-21 Identify failed requests and slow endpoints (5.2 Live network visualisation)
- [ ] T5-22 Timeline showing when requests occurred (5.2 Live network visualisation)
- [ ] T5-23 Compare network activity between runs (5.2 Live network visualisation)
- [ ] T5-24 Drag from a file/function to an unconnected API (5.3 Visual API integration)
- [ ] T5-25 Generate client modules and imports (5.3 Visual API integration)
- [ ] T5-26 Generate `.env` keys (5.3 Visual API integration)
- [ ] T5-27 OpenAPI / Swagger support (5.3 Visual API integration)
- [ ] T5-28 Support APIs without specifications (5.3 Visual API integration)
- [ ] T5-29 Preview generated code before applying changes (5.3 Visual API integration)
- [ ] T5-30 Ensure visual changes generate normal, readable Python (5.3 Visual API integration)
- [ ] T5-31 When removing a visual connection, offer to remove associated code (5.3 Visual API integration)
- [ ] T5-32 Detect changes to generated code and update graph relationships (5.3 Visual API integration)
- [ ] T5-33 Trace data between functions (5.4 Data-flow visualisation)
- [ ] T5-34 Visualise function inputs and outputs (5.4 Data-flow visualisation)
- [ ] T5-35 Show where variables originate (5.4 Data-flow visualisation)
- [ ] T5-36 Show transformations applied to data (5.4 Data-flow visualisation)
- [ ] T5-37 Highlight data dependencies (5.4 Data-flow visualisation)
- [ ] T5-38 Interactive pipeline graph (5.4 Data-flow visualisation)
- [ ] T5-39 Pandas DataFrame pipeline visualisation (5.4 Data-flow visualisation)
- [ ] T5-40 Inspect DataFrame shapes, columns and types (5.4 Data-flow visualisation)
- [ ] T5-41 Preview intermediate DataFrame contents (5.4 Data-flow visualisation)
- [ ] T5-42 Visualise data filtering, grouping and merging (5.4 Data-flow visualisation)
- [ ] T5-43 Export visual pipelines as ordinary Python (5.4 Data-flow visualisation)
- [ ] T5-44 Each block is an ordinary Python function; inputs and outputs come from its parameters, type hints and return value (5.4 Data-flow visualisation)
- [ ] T5-45 Placing a block inserts a readable call (plus import); wiring blocks passes one block's result to the next (5.4 Data-flow visualisation)
- [ ] T5-46 Built-in library of useful steps (read CSV/JSON, filter rows, group by, call API endpoint, retry, save) (5.4 Data-flow visualisation)
- [ ] T5-47 User blocks: mark a project function (e.g. `@block` or `.ideforfun/blocks/`) and it appears in the library (5.4 Data-flow visualisation)
- [ ] T5-48 Editing a block opens its Python; editing a built-in makes a project copy first, so library updates never overwrite it (5.4 Data-flow visualisation)
- [ ] T5-49 A single "for each" container block instead of loop/if wiring (5.4 Data-flow visualisation)
- [ ] T5-50 Live call graph while debugging (5.5 Execution visualisation)
- [ ] T5-51 Animated function execution (5.5 Execution visualisation)
- [ ] T5-52 Visualise recursion trees (5.5 Execution visualisation)
- [ ] T5-53 Show current execution position on the graph (5.5 Execution visualisation)
- [ ] T5-54 Display live variable values on nodes (5.5 Execution visualisation)
- [ ] T5-55 Track variable changes over time (5.5 Execution visualisation)
- [ ] T5-56 Visualise object relationships (5.5 Execution visualisation)
- [ ] T5-57 Inspect object properties without opening debugger panels (5.5 Execution visualisation)
- [ ] T5-58 Compare variable states between execution points (5.5 Execution visualisation)
- [ ] T6-01 Project templates and custom scaffolding (6.4 Developer automation)
- [ ] IDEA-01 Automatically reconstruct the architecture of an unfamiliar codebase (Architecture explorer)
- [ ] IDEA-02 Show components, dependencies, entry points and communication paths (Architecture explorer)
- [ ] IDEA-03 Provide a visual map of how the project works (Architecture explorer)
- [ ] IDEA-04 Show execution time directly on the project graph (Visual profiler)
- [ ] IDEA-05 Highlight expensive functions (Visual profiler)
- [ ] IDEA-06 Display call paths to explain where time is spent (Visual profiler)
- [ ] IDEA-07 Detect database connections and ORM models (Database explorer)
- [ ] IDEA-08 Generate interactive schema diagrams (Database explorer)
- [ ] IDEA-09 Show database relationships (Database explorer)
- [ ] IDEA-10 Navigate from schema elements to relevant code (Database explorer)
- [ ] IDEA-11 Track events through an application: where they originate, which handlers process them, what gets triggered and where they end up (Event-flow visualiser)
- [ ] IDEA-12 Support asynchronous and event-driven applications (Event-flow visualiser)
- [ ] IDEA-13 Visualise object references, memory allocations and object lifetimes (Memory explorer)
- [ ] IDEA-14 Inspect memory usage during execution (Memory explorer)
- [ ] IDEA-15 Identify unexpected object retention (Memory explorer)

## M5: Ship 1.0 (9)

- [ ] SHIP-01 Windows installer (per-user, no admin) and portable build
- [ ] SHIP-02 Linux AppImage and .deb (package `forge`, replaces `ideforfun`)
- [ ] SHIP-03 SHA256 checksums for every download
- [ ] SHIP-04 Updates: check after start and every 6 h, download in the background, install when Forge closes, "Update ready" restarts now
- [ ] SHIP-05 What's new on the Welcome page after an update; Help -> Release Notes
- [ ] SHIP-06 Settings -> Updates (keep Forge up to date) and Help -> Check for Updates
- [ ] SHIP-07 Import the Electron app's settings, keybindings, layouts, recent folders and templates on first run
- [ ] SHIP-08 CI on Windows and Linux (fmt, clippy, tests); release workflow
- [ ] SHIP-09 Performance budgets met and recorded (docs/BASELINE.md)

## Log

- 2026-11-08: ledger created from the old checklist (163 items).
- 2026-11-09: editor view (GPUI) on forge-text: T1-03, T1-04, T1-05, T1-06, T1-07, T1-09, T1-10, T1-11, T1-12, T1-13, T1-14, T1-15, T1-16, T2-02. 159 model tests + 13 headless view tests. Empty window 75 MB (debug).
- 2026-11-09: workspace shell: dock with the four tasks (layouts.json), tabs and split groups, file tree (create/rename/delete), palette (commands, files, line, choices, shortcut capture), Keyboard Shortcuts, Welcome page, status bar, save/close warnings, encodings, line endings, large/binary files, crash backups, workspace state. T1-01, T1-02, T1-08, T1-17..T1-21, T2-01, T2-06, T2-10, T2-11. Window with a project 77 MB (debug).
- 2026-11-09: find/replace bar, Settings dialog and project settings, integrated terminal (alacritty_terminal on ConPTY/pty: several terminals, shells, copy/paste, selection, history, restored per project). T2-03, T2-05, T2-07, T2-08, T2-09, T2-12, T2-13. Window with a project and a shell 129 MB (debug).
- 2026-11-09: Run with the Output panel, run configurations and tasks, Python environments (interpreter picker, .venv, packages, missing dependencies). T2-04, T2-14, T2-15, T2-16, T2-17. forge-run crate (real-Python tests).
- 2026-11-09: Tests panel (pytest via JUnit XML, unittest via -v output; run all / failed / file / at cursor; failures link to their line). T2-19. M2 done except T2-18 (updates: with M5's installer work).
- 2026-11-09: language server in the editor (basedpyright, installed into Forge's tools venv when missing): diagnostics with squiggles and the Problems panel, completion (auto-imports apply their import), hover, go to definition / type definition and back, references, implementations (subclasses and overrides), rename across files, symbols in the file and the project (palette search mode), call hierarchy, Outline, semantic highlighting, highlighted uses of the name under the cursor; auto-restart. T3-05, T3-06, T3-07, T3-08, T3-09, T3-14.
- 2026-11-09: code tools: Ruff warnings as you type (with a 💡 on lines that have a fix), Quick Fix (Ruff fixes, language-server actions, ignore a rule on a line, fix all), Format Document / Selection with Ruff or Black (and format on save), Organize Imports (Ruff, or the language server without Ruff). forge-core lint (Ruff JSON, smallest line edit, noqa) and forge-run tools (tested against a real Ruff). Fixed on the way: Ruff's --range ends at the start of its end line, so the old Format Selection skipped the last selected line. T3-01, T3-10, T3-11, T3-12, T3-13.
- Historical ruling (superseded by the M2/M3 completion below): T3-15 was deferred while the app had one server slot; the continuation introduced the services pool and independent integration processes.
- 2026-11-09: T3-03 complete: connected the existing Git status parser and file-tree decorations to a background worker. Refresh on folder open, save, create/rename/delete, manual tree refresh and window focus; poll every five seconds for terminal/external changes. Requests coalesce, changing projects cancels the old worker and clears decorations. Two headless regression tests failed before implementation (missing initial badges and stale badges after changing projects), then passed against a real temporary Git repository. `cargo test --workspace --offline`: 442 passed; `cargo clippy --workspace --all-targets --offline -- -D warnings`: passed. Changed Rust files pass rustfmt; workspace-wide fmt check still reports existing formatting differences elsewhere. Existing uncommitted Git-panel and theme work preserved. Next: connect the Git panel and implement the diff viewer (T3-16), then the remaining Git actions; debugger follows Git.
- 2026-11-09: M2/M3 continuation: Git panel connected to status, staging, commits/amend, branches and remote sync; native read-only diff/hunk, history graph, commit details and blame views added, with merge-resolution commands using editor undo. Keyboard commit conflict guard regression reproduced and fixed. Verification in progress; debugger and updates remain open.
- 2026-11-09: Git UI complete (T3-16..T3-21): connected the existing panel to background Git commands; diffs with coloured lines and stage/unstage/discard hunk actions; commits/amend and stage-all confirmation; local/remote branch selection and creation; merge conflict navigation and current/incoming/both resolution as undoable edits; virtualised history with a painted branch graph, commit-file inspection and per-line blame. Git generates untracked-file patches so spaces and missing final newlines survive staging. Status changes reload clean buffers in the background without overwriting intervening edits; project generations discard stale job results. Existing Git backend integration tests and new native panel/workspace tests pass.
- 2026-11-09: Debugger complete (T3-04, T3-22..T3-27): DAP request/response transport with timeouts and loopback-only child connections; debugpy launch/attach; integrated terminal requests for `input()`; continue/pause/step/restart/stop; sessions and threads, stack frames, expandable scopes/variables, console evaluation, dotted-name hover values, watches and exception filters. Breakpoints support conditions, hit counts, log messages and enabled state, follow edits/renames/deletes, and persist with watches/filters in the existing workspace format. Real debugpy integration tests cover conditional stopping, stack/scopes/variables, evaluation, stepping, terminal input, raised exceptions and multiprocessing attachment. A headless GPUI test also starts debugpy, types into the native terminal and verifies the paused source line, variables and watch value.
- 2026-11-09: T3-15 complete: `forge-lsp::services` owns multiple servers, routes documents/queries by their owner, merges diagnostics and project symbols, and syncs/flushes/saves each server's documents. The app has independent integration process/event-pump slots and generic editor routing; basedpyright remains the only bundled language integration. The two-server regression test initially exposed cross-language document leakage in `LspServer::sync_docs`; language filtering fixed it. Python rename validation and implementation fallbacks are integration hooks rather than restrictions on other servers.
- 2026-11-09: T2-18 complete: `forge-update` uses pinned Velopack 1.2.161 for installed-app checks, background download, install after all windows close, and restart through the normal unsaved-file close path. Automatic checks run after startup and every six hours when enabled; manual checks, download/restart choices, status-bar notices, Settings information and saved release notes/Welcome notices are connected. Source/portable copies offer the releases page. Version/release parsing and one-time upgrade notes have unit tests. Installer packaging, a published native feed, Windows/Linux tester rounds and the installed 1.0.0 -> 1.0.1 update-path test remain M5 work, not claimed as verified here. API reference: https://docs.rs/velopack/1.2.161/velopack/struct.UpdateManager.html.
- Ruling: use `https://github.com/louisatron84/forge-releases` as the native releases-only repository, overridable at build time with `FORGE_RELEASE_REPOSITORY`. This follows PLAN.md's public-feed option and avoids the frozen Electron feed; publishing/configuring the release host remains M5. Cost if changed: rebuild with the chosen repository URL.
- 2026-11-09: M2 and M3 implementation complete. Final checks: `cargo test --workspace --offline` **461 passed, 0 failed**, including real Git/Python/debugpy and headless GPUI checks; `cargo clippy --workspace --all-targets --offline -- -D warnings` passed; `cargo fmt --all -- --check` and `git diff --check` passed. Workspace formatting differences mentioned earlier have been resolved. Self-review completed without subagents, following this repo's no-subagents rule. Changes remain on `native` for owner review; no merge, release or publication performed. Next milestone: M4.
- 2026-11-09: Investigated the reported file creation/renaming and unexpected repository workspace. Current-source keyboard and mouse creation/rename flows passed before changes; the running `target/debug/forge.exe` was an older build (15:40), while tests had built a separate test executable. Startup also restored `forge-native` from the real recent-project list because reopening was on by default. A new startup regression test reproduced that unwanted automatic restoration. Per the user's requested behavior, reopening is now opt-in; explicit saved preferences and command-line folder selection still work. The Files panel now offers Open Folder / New Project Folder without an open workspace, an Open action with a workspace, and the current project path. Welcome uses the clearer New Project Folder label. Recent projects and their data are preserved.
- 2026-11-09: Added seven regression/interaction tests covering fresh startup, explicitly enabled restoration, standalone file saving, choosing/creating a project folder, mouse and keyboard file creation, editing the created document, and renaming with extension preservation. Workspace tests now include the same toolkit Root wrapper as the running app. `cargo test --workspace --offline`: **468 passed, 0 failed**; Clippy, rustfmt and diff checks passed. The normal build could not replace `forge.exe` because the user had it open; built the updated runnable copy as `target/debug/forge-current.exe` with `cargo rustc -p forge-app --offline --bin forge -- -o C:\Users\louis\forge-native\target\debug\forge-current.exe` (successful; rustc emitted the two expected custom-output-name warnings). Close the old app and use this copy, or rebuild normally after closing it. No manual app launch or unsaved-work interruption performed.
- 2026-11-09: The user's screenshot prompted a geometry-level reproduction of the empty orange name box. **Confirmed source bug:** in a 200px Files panel, the virtual list's name row shrank to its intrinsic width, leaving the actual text viewport **0px wide**. Previous keystroke tests could type into the focused but invisible input and therefore missed this. Added an explicit full-width constraint to the name row. A second failing assertion found its height was 36px in a 22px virtual row because the toolkit's `Input::h` only configures multiline inputs; setting the single-line frame height through `Styled` and removing vertical padding fixes overlap. The new Root-wrapped, 200px-wide regression asserts usable viewport dimensions and row height, then clicks, types, creates both a file and folder, and renames the folder. It failed before each fix and passes afterward. This supersedes treating the old executable as an explanation for the creation symptom.
- 2026-11-09: Name-box fix verified: `cargo test --workspace --offline` **469 passed, 0 failed**; Clippy with warnings denied, rustfmt and diff checks passed. `cargo build -p forge-app --offline` succeeded, and both `target/debug/forge.exe` and the previously shared `forge-current.exe` have been refreshed to the same fixed build. No manual app launch performed.

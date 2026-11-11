# Conventions for the Forge native rewrite

Read this before porting anything. The old app (Electron + TypeScript, frozen
at 0.50.0) is at `C:\Users\louis\IDEforfun` and is **read-only**. Its source and
its tests are the specification; `docs/parity/` lists every feature, and
`docs/PROGRESS.md` is the ledger of what is done.

## Crates and where code goes

| Crate | Holds | Depends on |
|---|---|---|
| `forge-core` | Pure logic from `src/shared/*.ts` and the pure parts of `src/renderer/**` (graph layout and view model, dock model, fuzzy matching, keymap parsing, where-you-are). No I/O, no processes, no GPUI. | serde, regex… |
| `forge-text` | The editor model: buffer, cursors, selection, undo, pairs, folding, find, tokenizers and highlighting, editor commands. No GPUI. | forge-core (only if needed) |
| `forge-sys` | File system and OS services: file ops, directory listing, project search, Git (the `git` CLI), user state and backups, project templates on disk, OpenAPI loading, test calls (HTTP), `.env` files. | forge-core |
| `forge-run` | Running Python: interpreter discovery, venvs and pip, run/trace spawning, the trace socket server, test runners, lint and format tools. | forge-core, forge-sys |
| `forge-scan` | Runs `python/projectScan.py` and `python/dataflow.py` and parses their JSON into forge-core types. | forge-core, forge-run |
| `forge-lsp` | The language-server client (JSON-RPC over stdio) and basedpyright setup. | forge-core |
| `forge-dap` | The debug-adapter client (debugpy), persistent breakpoints and child adapter connections. Uses the shared Content-Length framing and text line-marker model. | forge-core, forge-lsp, forge-text |
| `forge-term` | Terminal sessions: `portable-pty` plus the `alacritty_terminal` screen model. | — |
| `forge-update` | Updates (Velopack), release notes, What's new. | velopack, ureq, semver, serde |
| `forge-app` | Everything GPUI: the window, docking and tasks, every panel and view, themes, keymap and menus. The only crate that uses `gpui-kit`. | all of the above |

`python/` holds the Python helpers **copied unchanged** from the old repo; Rust
embeds them with `include_str!`.

## Porting rules

- **Behaviour is the spec, not the TypeScript's shape.** Port what the code
  does, including edge cases, limits, wording shown to the user and file
  formats. Write idiomatic Rust (enums, `Option`, `Result`, iterators); don't
  transliterate JavaScript idioms.
- **Port the tests first.** For each module, translate its `*.test.ts` cases
  into `#[test]`s (same names in snake_case, same inputs and expected values),
  watch them fail, then implement. A test you can't port (DOM-only) is noted
  in the module's doc comment with the reason.
- **Keep file formats compatible.** Anything the old app saved (settings,
  layouts, `.forge/*.json`, keybindings, recent folders, templates) must load
  in the Rust app. Use serde with `#[serde(rename_all = "camelCase")]` and the
  same field names; unknown fields are ignored, missing ones defaulted.
- **Tracer and scanner messages stay byte-compatible** with `python/*.py`.
- **Errors:** library crates return `Result<T, E>` with an error type per crate
  (`thiserror`), or `anyhow::Result` in `forge-app`. No `unwrap()`/`expect()`
  outside tests and provably-infallible spots (comment why).
- **No panics on bad input** from files, processes or the network: validate and
  report, like the TypeScript did.
- **Text positions:** lines and columns are 0-based like the old editor; LSP
  positions are UTF-16 code units (convert at the `forge-lsp` boundary).
- **Names:** modules in `snake_case` after the TS file (`projectGraph.ts` →
  `project_graph.rs`); types keep their TS names (`RawScan`, `ProjectGraph`).
- **Docs:** each module starts with a `//!` comment saying what it does and
  which TS file(s) it replaces.
- **Security rules carry over:** path checks (no `..`, stay inside the
  project), secret masking (URLs, headers, `.env`), tokens on local sockets.

## Checks every change must pass

```
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Real-Python tests skip themselves when no `python`/`python3` is on PATH, like
the old ones did.

UI views are tested headlessly with `gpui-kit`'s test support
(`#[gpui_kit::test]`, `gpui_kit::test::*`): render the view, drive clicks and
keys, assert state. Never launch the app to check something by hand.

## Branches and the ledger

Work happens on the `native` branch, committed in small steps and pushed
often; pull requests into `main` are merged by the owner. When a feature is
done (implemented, tests ported and passing), tick its line in
`docs/PROGRESS.md` and add a line to the Log at the bottom. The ledger is how
work resumes after a break: trust it and `git log` over memory.

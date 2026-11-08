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

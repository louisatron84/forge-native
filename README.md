# Forge (native)

Forge rebuilt as a native Rust app with [GPUI](https://www.gpui.rs/), replacing
the Electron version ([IDEforfun](https://github.com/louisatron84/IDEforfun),
frozen at 0.50.0). The goal is the same Forge, much faster and lighter: no new
features until this version does everything 0.50.0 does.

Private and proprietary: see [LICENSE](LICENSE).

## Plan

[docs/PLAN.md](docs/PLAN.md) has the architecture, the performance budgets and
the milestones (M0 foundations and go/no-go gate, M1 editor, M2 IDE
foundations, M3 language and tooling, M4 visual Forge, M5 ship 1.0).

## Build

Needs Rust (stable) and, on Windows, the Visual Studio C++ Build Tools.

```
cargo run -p forge-app
cargo test --workspace
```

## Installers and website downloads

See [docs/DISTRIBUTION.md](docs/DISTRIBUTION.md) for the manual native build
workflow, Windows setup/portable downloads, Linux Debian/portable packages,
macOS Apple Silicon/Intel disk images, signing and website download metadata.

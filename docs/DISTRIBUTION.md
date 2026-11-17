# Website downloads

Run **Build website downloads** manually in GitHub Actions. It builds on Windows,
Ubuntu 22.04 and macOS (Apple Silicon and Intel). Download the combined
`forge-website-downloads` artifact. Nothing is published automatically.

Upload its contents together to your website's downloads directory:

| Platform | Download |
| --- | --- |
| Windows x64 | `Forge-win-x64-Setup.exe` |
| Windows portable | `Forge-win-x64-Portable.zip` |
| Linux x64 AppImage | generated `*.AppImage` (exact filename in downloads.json) |
| Linux x64 Debian/Ubuntu | `Forge-<version>-linux-x64.deb` |
| Linux x64 portable | `Forge-<version>-linux-x64.tar.gz` |
| macOS Apple Silicon | `Forge-<version>-macos-arm64.dmg` |
| macOS Intel | `Forge-<version>-macos-x64.dmg` |

`downloads.json` supplies filenames, OS, architecture, byte sizes, signing status
and SHA-256 hashes for website download buttons. `SHA256SUMS` lets users verify
downloads. Metadata is validated against the actual files when combined.

Build locally with Python 3.11+ and Rust on the matching operating system:
The workflow pins Rust 1.99.1, matching the verified Windows toolchain.

```
python scripts/package.py --target x86_64-pc-windows-msvc --velopack
python scripts/package.py --target x86_64-unknown-linux-gnu --velopack
python scripts/package.py --target aarch64-apple-darwin
python scripts/package.py --target x86_64-apple-darwin
```

Install the pinned update packager with `dotnet tool install --global vpk --version
1.3.39-g8d2844f` (requires .NET 9). The optional standalone Windows path, without
`--velopack`, requires NSIS 3 (`choco install nsis`). Linux requires the build packages
listed in the workflow and `dpkg-deb`. macOS requires Xcode command-line tools.
Fonts and Python inspection helpers are embedded in the executable. Python and
Git are user-installed runtime tools; Forge discovers them on the user's machine.
The portable Linux archive needs the same shared libraries listed by the Debian
package. Linux ARM builds are not currently included.

Windows installs per user, creates a Start menu shortcut and
registers an uninstaller. Uninstall keeps the user's projects and settings.
Both Windows package paths include the release Visual C++ runtime beside the
executable, following [Microsoft's app-local deployment guidance](https://learn.microsoft.com/cpp/windows/choosing-a-deployment-method).
They do not request a system runtime installation. The packager locates Visual
Studio's `VC/Redist/MSVC/<version>/x64/Microsoft.VC*.CRT` directory with `vswhere`;
set `FORGE_VC_REDIST` to the version directory if detection is unavailable. Missing
release runtime files fail the build. Debug runtime files are never selected.
Linux AppImage updates replace the image
when Forge exits; Debian/portable installs use the release download link.
Linux installs `/usr/bin/forge` and a desktop entry. On Mac, drag Forge.app to
Applications from the mounted disk image.

The default Mac build is ad-hoc signed. For public distribution, run the script
on a Mac with a Developer ID certificate installed and set `MACOS_SIGN_IDENTITY`.
Set `MACOS_NOTARY_PROFILE` to a previously stored `notarytool` keychain profile to
submit and staple the DMG. The script fails if requested notarization fails.
The hosted workflow deliberately has no signing credentials or publication step.
Windows binaries are unsigned until a Windows code-signing certificate is wired
into the release process. Signing status is recorded in the download manifest.

Set `workspace.package.version` before building an official release. Current
`1.0.0-dev` packages are development builds. `--velopack` produces
`releases.win-x64.json` / `releases.linux-x64.json`, `assets.*.json` and `.nupkg`
files alongside the installers. Upload these together to the public releases-only
repository, with filenames intact. Channels are recorded in each installed
manifest and separate Windows/Linux package names. Run
`python scripts/verify_release.py releases` before uploading; it checks website
metadata and update-feed hashes. Public releases are still published by the owner;
the build workflow only uploads private CI artifacts. Development previews must be
marked prerelease and do not become stable update candidates.

Use a clean output directory for each version (`--output releases/<version>`).
The installed upgrade path still requires separate release validation. macOS/Linux artifacts
must pass their native build jobs and installation tests before being advertised
as supported public releases.

## Validation status (2026-11-17)

The [native packaging run](https://github.com/louisatron84/forge-native/actions/runs/43628447232)
at commit `19ef511d005478760a208a2cef4c07083df360ac` built Windows x64,
Linux x64, macOS ARM64 and macOS x64 successfully and combined their artifacts.
The downloaded bundle in `releases/m5-website` passes local verification of all
13 download records, every update-feed package hash and every `SHA256SUMS` entry.
This is build and archive evidence for that commit, not installation validation
or proof that the latest source has passed the native build jobs.

The [2026-11-17 packaging run](https://github.com/louisatron84/forge-native/actions/runs/44149591448)
at `1985093` also passed Windows x64 and Linux x64 build/feed verification and
hosted install/uninstall checks. Windows checks confirmed the installed and
portable executable hashes, bundled runtime DLL, and absence of the vcredist
bootstrap dependency. Debian installation checked the binary, desktop entry and
icon. Both uninstall checks preserved a marker in Forge's user-data directory.
These checks do not launch the desktop UI or validate an upgrade from another
version. The path-identity corrections at `4a01eab` passed both Windows and Linux
[CI jobs](https://github.com/louisatron84/forge-native/actions/runs/44155599679)
and their native installer jobs; the Intel Mac build result is not yet recorded.

The installed Windows update fixture can run independently using the workflow's
`upgrade_only` option and a `package_run_id` containing a verified Windows artifact.
It builds a console fixture with the optional `forge-update/release-validation`
feature, packages it as the separate `ForgeUpgradeSmoke` application at 1.0.0 and
1.0.1, and uses Forge's own download, persisted notes and apply-on-exit methods.
The controller verifies that files stay at 1.0.0 while the owner is alive, become
1.0.1 after exit, and preserve user data through update and uninstall. Results
and logs are private CI artifacts. Normal Windows packaging also runs this check.
The script refuses personal and self-hosted machines. This fixture does not test
GPUI restart, unsaved-file prompts or the Welcome screen; those remain desktop
release gates. It does not publish either test version to the native feed.

The following release gates remain open:

- Windows clean-user installation, portable launch, uninstall preserving user
  data, and installation on a machine without the Visual C++ runtime. Packaging
  now bundles the runtime locally; no-admin installation still needs validation
  on a clean machine. The earlier downloaded bundle uses the old prerequisite.
- Linux AppImage launch on X11/Wayland, Debian installation/replacement of
  `ideforfun`, dependency resolution and uninstall preserving user data.
- Installed upgrade from one test version to the next, including unsaved-file
  cancellation, apply on close, restart and the one-time What's new notice.
- Windows/Linux test workflow success for the final source revision, reference
  desktop performance measurements, and the tester round.
- Owner signing/publication and the Electron/website handover described in
  `docs/PLAN.md`. Nothing is published by the build workflow.

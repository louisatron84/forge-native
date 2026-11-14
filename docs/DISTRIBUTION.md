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
The Velopack setup handles the Visual C++ runtime prerequisite; runtime installation
may require elevation if it is missing. The optional standalone NSIS build requires
the runtime to be installed separately. Linux AppImage updates replace the image
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

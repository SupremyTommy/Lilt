# Handoff for the next AI

## Project

Lilt is a native Windows music player written in Rust and egui. The Rust project is this folder; the Python app lives one directory above it. The GitHub remote is `https://github.com/ultimatebloxstudium-tech/Lilt.git`.

## Current work

Version v0.2.13 contains the following MSI-only release changes:

- Added streaming Ogg Opus decoding via `opus-pure`, including Opus streams stored in `.ogg` files. Seeking restarts and skips decoded samples because the codec reader has no arbitrary seek API.
- Added a quiet startup check against the latest published GitHub release. If a newer release has a Windows x64 MSI, Lilt asks first; Update now downloads it and launches `msiexec`, while Later does nothing. Network/check failures are silent.
- Windows system media-key commands are processed while Lilt is unfocused, and playback state is still published to the Windows media session.
- Settings now write via a temporary file and atomic rename, with an executable-adjacent data folder fallback (then temp) if Windows denies the normal Roaming/Local AppData locations. This also handles Codex's sandboxed AppData paths. Startup loads the newest valid settings copy.
- MSI registers Lilt as a selectable app for MP3, FLAC, WAV, M4A, OGG, OPUS, and AAC. A first-run prompt opens Windows Default Apps settings; Windows itself must apply the choice.
- MSI adds “Open folder with Lilt” to the File Explorer folder-background menu. It passes the current folder to Lilt, which adds it to the library and scans it. Opening an associated audio file scans its parent folder and starts playback.
- MSI UI was switched to WiX `WixUI_Minimal` to remove the end-user license agreement screen. Publish only the MSI on GitHub; the normal local EXE and portable ZIP build artifacts remain available, and the EXE is also installed by the MSI.

The follow-up source passed `cargo check` and `cmd /c build-release.bat` completed for local v0.2.13 artifacts:

- `dist/Lilt-v0.2.13-windows-x64.msi` (local installer for user testing)
- `dist/Lilt-v0.2.13-windows-x64.zip` (local portable package)
- `target/release/lilt-player.exe` (local build output)

The user installed and tested the v0.2.13 MSI and reported that everything worked perfectly. It is published at https://github.com/ultimatebloxstudium-tech/Lilt/releases/tag/v0.2.13 with exactly one asset: `Lilt-v0.2.13-windows-x64.msi`. The EXE and ZIP remain local build outputs. The v0.2.13 release tag points to commit `b2d3b07ef2ae077f58195b6b7ded1e0103ff6db5`.

During development, playback launched from the Codex terminal exited with `STATUS_INVALID_HANDLE` for Opus and FLAC, although a direct decoder probe decoded 11,171 packets successfully. The user subsequently installed and tested the MSI and reported that it all worked perfectly. They plan to test the updater from a second Windows 11 device running v0.2.12; with v0.2.13 published, that device should prompt to download the MSI.

No always-on logon process is required for Explorer integration. The MSI registers the context menu and default-app capabilities in per-user registry keys; Explorer reads those registrations without Lilt running at login.

Release `v0.2.12` is published with fixes for borderless window resizing and click handling:

- Smart Mixes has a dedicated icon at `assets/icons/ic-smart-mixes-24px.svg`.
- The Smart Mixes page and navigation already use vertical scrolling.
- Mix Studio can send compact title, artist, genre, and familiarity metadata for the full eligible local library to Google AI.
- Mix options include saved Liked, Played, and Unplayed filters, an All music shortcut, model, thinking level, and mix size. Disliked and excluded tracks are always filtered out.
- Both context-menu reveal actions now use Windows Shell APIs to open the containing folder and select the track, rather than passing `/select,...` through Explorer's command-line parser (which could open Desktop).
- The app now uses a custom borderless top bar with a grouped Menu dropdown, minimize/maximize/close buttons, drag-to-move, and edge resize grips. It displays `♫ LILT · SupremyTommy` persistently in the title bar.
- Startup splash shows white `SupremyTommy` on `#252E3B`, tweens to black for the first 0.25 seconds, holds until 0.5 seconds, then fades out to the app over 0.35 seconds.
- Removed the full-window resize overlay that blocked clicks. Resizing now starts only when pressing within the 8px window edge. Added a broad draggable area in the title bar.
- AI calls are opt-in/on demand; the app sends no file paths or audio.
- Version references are now 0.2.12 in Cargo.toml, Cargo.lock, build-release.bat, installer/Package.wxs, and README.md.

## Build status

For v0.2.8, `cmd /c build-release.bat` completed successfully and produced:

- `dist/Lilt-v0.2.8-windows-x64.msi`
- `dist/Lilt-v0.2.8-windows-x64.zip` (portable app package containing `lilt-player.exe`)
- `target/release/lilt-player.exe`

`cargo test` passed all 24 tests after the window interaction fix. `cargo run` was launched in debug mode and the user confirmed the behavior is fixed. `cmd /c build-release.bat` completed successfully for v0.2.12.

`cargo check` also passed. It reports an existing dead-code warning for `scan_folders`; release compilation reports the same warning. Build artifacts are ignored/untracked under `dist` and `target`.

## GitHub release status

Release `v0.2.12` is published at https://github.com/ultimatebloxstudium-tech/Lilt/releases/tag/v0.2.12. GitHub verified the standalone EXE, MSI, and portable ZIP are uploaded. The release commit is `cae2e42`; the annotated tag is `v0.2.12`. GitHub CLI device login completed successfully as `ultimatebloxstudium-tech` with `repo` scope.

The user's instruction is to publish the GitHub release through the terminal and to leave this handoff file in the Rust project folder.

## Working tree cautions

There are timestamped `src/main.backup-*.rs` files and `src/main.rs.backup` already in the working tree. Do not include these backup files in the release commit. Review `git status` and stage only the intended source, release metadata, README, and this handoff note.

# Handoff for the next AI

## Project

Lilt is a native Windows music player written in Rust and egui. The Rust project is this folder; the Python app lives one directory above it. The GitHub remote is `https://github.com/ultimatebloxstudium-tech/Lilt.git`.

## Current work

Release `v0.2.11` is published. The v0.2.12 candidate fixes borderless window resizing and click handling:

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

`cargo test` passed all 24 tests after the window interaction fix. `cargo run` was launched in debug mode and the user confirmed the behavior is fixed. Build v0.2.12 with `cmd /c build-release.bat` before publishing.

`cargo check` also passed. It reports an existing dead-code warning for `scan_folders`; release compilation reports the same warning. Build artifacts are ignored/untracked under `dist` and `target`.

## GitHub release status

Release `v0.2.11` is published at https://github.com/ultimatebloxstudium-tech/Lilt/releases/tag/v0.2.11. GitHub verified the standalone EXE, MSI, and portable ZIP are uploaded. The release commit is `90f9e16`; the annotated tag is `v0.2.11`. GitHub CLI device login completed successfully as `ultimatebloxstudium-tech` with `repo` scope. Publish the tested v0.2.12 artifacts next.

The user's instruction is to publish the GitHub release through the terminal and to leave this handoff file in the Rust project folder.

## Working tree cautions

There are timestamped `src/main.backup-*.rs` files and `src/main.rs.backup` already in the working tree. Do not include these backup files in the release commit. Review `git status` and stage only the intended source, release metadata, README, and this handoff note.

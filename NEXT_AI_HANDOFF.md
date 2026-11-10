# Handoff for the next AI

## Project

Lilt is a native Windows music player written in Rust and egui. The Rust project is this folder; the Python app lives one directory above it. The GitHub remote is `https://github.com/ultimatebloxstudium-tech/Lilt.git`.

## Current work

This checkout is being prepared for release `v0.2.8`:

- Smart Mixes has a dedicated icon at `assets/icons/ic-smart-mixes-24px.svg`.
- The Smart Mixes page and navigation already use vertical scrolling.
- Mix Studio can send compact title, artist, genre, and familiarity metadata for the full eligible local library to Google AI.
- Mix options include saved Liked, Played, and Unplayed filters, an All music shortcut, model, thinking level, and mix size. Disliked and excluded tracks are always filtered out.
- AI calls are opt-in/on demand; the app sends no file paths or audio.
- Version references were bumped from 0.2.7 to 0.2.8 in Cargo.toml, Cargo.lock, build-release.bat, installer/Package.wxs, and README.md.

## Build status

`cmd /c build-release.bat` completed successfully. It produced:

- `dist/Lilt-v0.2.8-windows-x64.msi`
- `dist/Lilt-v0.2.8-windows-x64.zip` (portable app package containing `lilt-player.exe`)
- `target/release/lilt-player.exe`

`cargo check` also passed. It reports an existing dead-code warning for `scan_folders`; release compilation reports the same warning. Build artifacts are ignored/untracked under `dist` and `target`.

## GitHub release status

The latest published tag is `v0.2.7`; `v0.2.8` did not exist when checked. GitHub CLI device login completed successfully as `ultimatebloxstudium-tech` with `repo` scope. The release commit is local, `main` is one commit ahead of `origin/main`, and fetch found no remote changes. Push the release commit and tag, then attach the MSI, standalone EXE, and ZIP through `gh release create`.

The user's instruction is to publish the GitHub release through the terminal and to leave this handoff file in the Rust project folder.

## Working tree cautions

There are timestamped `src/main.backup-*.rs` files and `src/main.rs.backup` already in the working tree. Do not include these backup files in the release commit. Review `git status` and stage only the intended source, release metadata, icon, README, and this handoff note.

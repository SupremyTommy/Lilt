# Lilt Player (Rust)

![Lilt app icon](assets/app_icon.png)

Native Windows desktop player. It does not need Python or a Python runtime.

## Build

Install the stable Rust toolchain with the MSVC target and run `build-release.bat`.
The optimized Windows executable is written to `target\release\lilt-player.exe` without a console window. The script creates `dist\Lilt-v0.2.4-windows-x64.zip` as a portable package and `dist\Lilt-v0.2.4-windows-x64.msi` as a per-user installer. The MSI wizard lets you choose optional Start Menu and Desktop shortcuts.

The release script installs WiX Toolset 7 locally if needed. On a fresh setup, read and accept its EULA once by following the [WiX EULA instructions](https://docs.firegiant.com/wix/osmf/) before building the MSI.

## Current features

- Background folder scanning, metadata, embedded album artwork, and library search
- Home and Discover pages with personalized recommendations, genre and decade mixes, and artist radio
- File, Playback, View, Help, and Playlists menus; playlists remain available in the sidebar
- Larger YouTube Music-inspired layout with Songs, Albums, Artists, Likes, Playlists,
  History, Disliked, Lyrics, and Settings navigation
- Grid, details, and list views; customizable accent, discovery balance, recommendation variety and
  size, unplayed-song priority, and library folders
- Persistent song likes/dislikes, followed artists, saved albums, playlists, history, and preferences
- Home Recommendations and New to you shelves personalized from liked music and listening history,
  with controls for discovery balance, unplayed-song priority, variety, and mix size
- Queue panel with reordering, play/pause/next/previous, keyboard shortcuts, shuffle, and repeat;
  album and artist collection shuffle randomizes the complete collection
- Expanded Now Playing view with album art, Up next, contextual For you picks, and one-click radio
  playback built from your library, listening history, likes, and current track
- Right-click track, album, artist, Home mix, or playlist items for playback, queueing, library,
  navigation, and collection actions; playlist menus also support rename and duplicate
- Windows multimedia keys for play/pause, next, previous, and stop, with current-track metadata
  shared with the system media overlay
- Redesigned bottom player with clickable album art, clear like/dislike controls, and a seek timeline;
  album art opens the expanded player. Space/Ctrl+P toggles playback and Left/Right changes tracks
  when the app is focused and no text field is active; queued songs advance while unfocused
- Toolbar and navigation icons load from SVG assets at runtime
- `.lrc` sidecar lyrics display; removing a track from the library preserves its audio file
- Collaborative artist credits and album-title grouping, with placeholder artist/album metadata omitted

Settings and library preferences are stored in the user's application configuration directory.
The first launch prompts for a music folder; additional folders can be managed in Settings.

## Still to migrate

The Python app remains available in the parent folder. System tray and mini-player modes,
watched-folder rescanning, embedded-lyrics extraction, tag editing, duplicate detection, and the
advanced Python player features have not yet been ported to this Rust version.

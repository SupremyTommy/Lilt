# Lilt Player (Rust)

Native Windows desktop player. It does not need Python or a Python runtime.

## Build

Install the stable Rust toolchain with the MSVC target and run `build-release.bat`.
The optimized standalone executable is written to `target\release\lilt-player.exe`.

## Current features

- Background folder scanning, metadata, embedded album artwork, and library search
- Larger YouTube Music-inspired layout with Home, Songs, Albums, Artists, Likes, Playlists,
  History, Disliked, Lyrics, and Settings navigation
- Grid, details, and list views; customizable accent, discovery balance, recommendation variety and
  size, unplayed-song priority, and library folders
- Persistent song likes/dislikes, followed artists, saved albums, playlists, history, and preferences
- Home Recommendations and New to you shelves personalized from liked music and listening history,
  with controls for discovery balance, unplayed-song priority, variety, and mix size
- Queue panel with reordering, play/pause/next/previous, keyboard shortcuts, shuffle, and repeat;
  album and artist collection shuffle randomizes the complete collection
- Right-click track, album, artist, Home mix, or playlist items for playback, queueing, library,
  navigation, and collection actions; playlist menus also support rename and duplicate
- Windows multimedia keys for play/pause, next, previous, and stop, with current-track metadata
  shared with the system media overlay
- Compact bottom player bar with a seek timeline; Space/Ctrl+P toggles playback and Left/Right
  changes tracks when the app is focused and no text field is active; Ctrl+F or `/` focuses search
- `.lrc` sidecar lyrics display; removing a track from the library preserves its audio file
- Collaborative artist credits and album-title grouping, with placeholder artist/album metadata omitted

Settings and library preferences are stored in the user's application configuration directory.
The first launch prompts for a music folder; additional folders can be managed in Settings.

## Still to migrate

The Python app remains available in the parent folder. System tray and mini-player modes,
watched-folder rescanning, embedded-lyrics extraction, tag editing, duplicate detection, and the
advanced Python player features have not yet been ported to this Rust version.

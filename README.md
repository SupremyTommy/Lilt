# Lilt

![Lilt app icon](assets/app_icon.png)

Lilt is a Windows music player for your local collection. Browse songs, albums, and artists, make mixes, and keep your library organized in one place. Your music files stay on your PC.

[Download the latest Windows installer](https://github.com/SupremyTommy/Lilt/releases/latest)

## Get started

1. Download and run the MSI installer.
2. Choose whether to add Start Menu and Desktop shortcuts, File Explorer integration, and Windows audio-file registration.
3. Choose an install folder, then click **Install**.
4. On the finish screen, leave **Launch Lilt** checked to open the app.
5. Choose a music folder when Lilt starts.

Registering audio files makes Lilt available in Windows' **Open with** and default-app choices. Windows still controls which app is the default.

## What you can do

- Browse and search songs, albums, artists, and playlists in grid, list, or details views.
- Build mixes from your library, explore recommendations, and use artist radio.
- Manage the queue, shuffle and repeat playback, and revisit listening history and stats.
- Save likes, dislikes, followed artists, albums, and playlists.
- Use `.lrc` lyrics and optional album-cover or artist-photo lookup.
- Customize themes and playback, library, and discovery settings. Settings save automatically, except sliders, which have a Save button.
- Keep a song, playlist, or album ready as background music, with optional looping and smart ducking when other apps play audio.
- Start Lilt hidden in the background, optionally at Windows login, and reopen Lilt to show the window again.
- Use **Quick Play with Lilt** from File Explorer to open just one audio file or folder. Quick Play asks before adding that folder to your library and scanning it.
- Use optional Google AI features for library questions, recommendations, and music mixes.

Google AI features require your own Google AI Studio API key, stored in Windows Credential Manager. Lilt sends music metadata needed for the requested feature; it does not send audio files or file paths.

## Build from source

Install the stable Rust toolchain with the MSVC target, then run `build-release.bat` from this folder. The script builds the app, a portable ZIP, and the MSI installer. It installs WiX Toolset locally if needed.

## Background and Quick Play

Background music and startup settings live under **Settings → Startup & background**. You can choose a source (current queue, current song, a playlist, an album, or a searched song), enable looping, start Lilt with Windows, and hide the window on launch. Reopening Lilt brings the existing window back instead of starting a second copy.

Smart ducking watches Windows audio sessions. When another app starts playing sound, Lilt lowers its own volume smoothly, then restores it about a second after the other audio stops. Detection is per app, so paused tabs and system notification sounds do not keep the music quiet.

Quick Play is added to Explorer context menus from **Settings → Startup & background**. It opens only the selected file or folder without loading or scanning your library, then asks whether to add a folder and scan it now. On Windows 11 it may appear under **Show more options**.

## Not included yet

A notification-area tray icon and mini-player modes, tag editing, and automatic rescanning of watched folders are still being developed.

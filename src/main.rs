use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    fs::{self, File},
    io::BufReader,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender, TryRecvError},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use directories::ProjectDirs;
use eframe::egui::{self, Color32, RichText, TextureHandle, Vec2};
use lofty::{prelude::*, probe::Probe};
use rodio::{Decoder, OutputStream, Sink, Source};
use serde::{Deserialize, Serialize};
use souvlaki::{MediaControlEvent, MediaControls, PlatformConfig};

const APP_NAME: &str = "Lilt";
const AUDIO_EXTENSIONS: &[&str] = &["mp3", "flac", "wav", "m4a", "ogg", "opus", "aac", "wma"];
const ACCENTS: [(&str, Color32); 3] = [
    ("Blue", Color32::from_rgb(54, 97, 255)),
    ("Sky", Color32::from_rgb(0, 162, 255)),
    ("Ice", Color32::from_rgb(204, 205, 211)),
];

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Track {
    path: PathBuf,
    title: String,
    artist: String,
    album: String,
    genre: String,
    year: u32,
    added: u64,
    duration: f64,
    #[serde(skip)]
    artwork: Option<Vec<u8>>,
    #[serde(skip)]
    search_text: String,
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct Playlist {
    name: String,
    tracks: Vec<PathBuf>,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
enum ViewMode {
    Grid,
    Details,
    #[default]
    List,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
enum RepeatMode {
    #[default]
    Off,
    All,
    One,
}

#[derive(Clone, Serialize, Deserialize)]
struct Preferences {
    #[serde(default)]
    folders: Vec<PathBuf>,
    #[serde(default)]
    liked_songs: HashSet<PathBuf>,
    #[serde(default)]
    disliked_songs: HashSet<PathBuf>,
    #[serde(default)]
    liked_artists: HashSet<String>,
    #[serde(default)]
    liked_albums: HashSet<String>,
    #[serde(default)]
    playlists: Vec<Playlist>,
    #[serde(default)]
    history: Vec<PathBuf>,
    #[serde(default)]
    excluded_tracks: HashSet<PathBuf>,
    #[serde(default)]
    view_mode: ViewMode,
    #[serde(default)]
    album_view_mode: ViewMode,
    #[serde(default)]
    artist_view_mode: ViewMode,
    #[serde(default)]
    accent: String,
    #[serde(default)]
    shuffle: bool,
    #[serde(default)]
    repeat: RepeatMode,
    #[serde(default = "default_discovery")]
    discovery: f32,
    #[serde(default = "default_prefer_unplayed")]
    prefer_unplayed: bool,
    #[serde(default = "default_recommendation_variety")]
    recommendation_variety: f32,
    #[serde(default = "default_recommendation_size")]
    recommendation_size: usize,
    #[serde(default = "default_ui_scale")]
    ui_scale: f32,
    #[serde(default = "default_volume")]
    volume: f32,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            folders: Vec::new(),
            liked_songs: HashSet::new(),
            disliked_songs: HashSet::new(),
            liked_artists: HashSet::new(),
            liked_albums: HashSet::new(),
            playlists: Vec::new(),
            history: Vec::new(),
            excluded_tracks: HashSet::new(),
            view_mode: ViewMode::default(),
            album_view_mode: ViewMode::default(),
            artist_view_mode: ViewMode::default(),
            accent: String::new(),
            shuffle: false,
            repeat: RepeatMode::default(),
            discovery: default_discovery(),
            prefer_unplayed: default_prefer_unplayed(),
            recommendation_variety: default_recommendation_variety(),
            recommendation_size: default_recommendation_size(),
            ui_scale: default_ui_scale(),
            volume: default_volume(),
        }
    }
}

fn default_discovery() -> f32 {
    0.5
}

fn default_prefer_unplayed() -> bool {
    true
}

fn default_recommendation_variety() -> f32 {
    0.65
}

fn default_recommendation_size() -> usize {
    40
}

fn default_ui_scale() -> f32 {
    100.0
}

fn default_volume() -> f32 {
    1.0
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Home,
    Songs,
    Albums,
    Artists,
    LikedSongs,
    LikedArtists,
    LikedAlbums,
    Playlists,
    History,
    Disliked,
    Lyrics,
    Settings,
    ArtistDetail,
    AlbumDetail,
}

struct ScanResult {
    tracks: Vec<Track>,
    error: Option<String>,
}

struct ArtworkResult {
    path: PathBuf,
    size: [usize; 2],
    rgba: Option<Vec<u8>>,
}

#[derive(Clone, Copy)]
enum MediaKeyCommand {
    Toggle,
    Play,
    Pause,
    Next,
    Previous,
    Stop,
}

struct HomeCard {
    title: String,
    subtitle: String,
    cover_index: usize,
    tracks: Vec<usize>,
    start: usize,
}

struct PlayerApp {
    tracks: Vec<Track>,
    folders: Vec<PathBuf>,
    liked_songs: HashSet<PathBuf>,
    disliked_songs: HashSet<PathBuf>,
    liked_artists: HashSet<String>,
    liked_albums: HashSet<String>,
    playlists: Vec<Playlist>,
    history: Vec<PathBuf>,
    excluded_tracks: HashSet<PathBuf>,
    query: String,
    page: Page,
    loaded_collection_page: Option<Page>,
    loaded_collection_query: String,
    loaded_collection_items: usize,
    view_mode: ViewMode,
    album_view_mode: ViewMode,
    artist_view_mode: ViewMode,
    accent: String,
    discovery: f32,
    prefer_unplayed: bool,
    recommendation_variety: f32,
    recommendation_size: usize,
    ui_scale: f32,
    volume: f32,
    home_genre: String,
    recommendations: Vec<usize>,
    recommendations_dirty: bool,
    latest_tracks: Vec<usize>,
    home_albums: Vec<(String, Vec<usize>)>,
    home_shelf_offsets: HashMap<String, f32>,
    shuffle: bool,
    repeat: RepeatMode,
    playlist_name: String,
    renaming_playlist: Option<usize>,
    playlist_rename_draft: String,
    selected_playlist: Option<usize>,
    selected_artist: Option<String>,
    selected_album: Option<String>,
    settings_draft: Option<Preferences>,
    pending_delete: Option<(PathBuf, String)>,
    lyrics: Option<String>,
    lyrics_error: Option<String>,
    queue: Vec<usize>,
    queue_collapsed: bool,
    current: Option<usize>,
    playback_offset: Duration,
    sink: Option<Sink>,
    stream: Option<OutputStream>,
    scan_receiver: Option<Receiver<ScanResult>>,
    scan_status: String,
    scan_busy: bool,
    status: String,
    textures: BTreeMap<PathBuf, TextureHandle>,
    texture_usage: VecDeque<PathBuf>,
    artwork_receiver: Receiver<ArtworkResult>,
    artwork_sender: Sender<ArtworkResult>,
    artwork_pending: HashSet<PathBuf>,
    artwork_failed: HashSet<PathBuf>,
    media_key_receiver: Receiver<MediaKeyCommand>,
    media_key_sender: Sender<MediaKeyCommand>,
    #[cfg(target_os = "windows")]
    media_controls: Option<MediaControls>,
    #[cfg(target_os = "windows")]
    media_controls_initialized: bool,
    #[cfg(target_os = "windows")]
    published_media_state: Option<(Option<PathBuf>, bool)>,
}

impl PlayerApp {
    fn new() -> Self {
        let (preferences, preference_error) = match load_preferences() {
            Ok(preferences) => (preferences, None),
            Err(error) => (Preferences::default(), Some(error)),
        };
        let (artwork_sender, artwork_receiver) = mpsc::channel();
        let (media_key_sender, media_key_receiver) = mpsc::channel();
        let mut app = Self {
            tracks: Vec::new(),
            folders: preferences.folders,
            liked_songs: preferences.liked_songs,
            disliked_songs: preferences.disliked_songs,
            liked_artists: preferences.liked_artists,
            liked_albums: preferences.liked_albums,
            playlists: preferences.playlists,
            history: preferences.history,
            excluded_tracks: preferences.excluded_tracks,
            query: String::new(),
            page: Page::Home,
            loaded_collection_page: None,
            loaded_collection_query: String::new(),
            loaded_collection_items: 50,
            view_mode: preferences.view_mode,
            album_view_mode: preferences.album_view_mode,
            artist_view_mode: preferences.artist_view_mode,
            accent: if preferences.accent.is_empty() {
                "Blue".into()
            } else {
                preferences.accent
            },
            discovery: preferences.discovery.clamp(0.0, 1.0),
            prefer_unplayed: preferences.prefer_unplayed,
            recommendation_variety: preferences.recommendation_variety.clamp(0.0, 1.0),
            recommendation_size: preferences.recommendation_size.clamp(20, 100),
            ui_scale: preferences.ui_scale.clamp(75.0, 150.0),
            volume: preferences.volume.clamp(0.0, 1.0),
            home_genre: String::new(),
            recommendations: Vec::new(),
            recommendations_dirty: true,
            latest_tracks: Vec::new(),
            home_albums: Vec::new(),
            home_shelf_offsets: HashMap::new(),
            shuffle: preferences.shuffle,
            repeat: preferences.repeat,
            playlist_name: String::new(),
            renaming_playlist: None,
            playlist_rename_draft: String::new(),
            selected_playlist: None,
            selected_artist: None,
            selected_album: None,
            settings_draft: None,
            pending_delete: None,
            lyrics: None,
            lyrics_error: None,
            queue: Vec::new(),
            queue_collapsed: false,
            current: None,
            playback_offset: Duration::ZERO,
            sink: None,
            stream: None,
            scan_receiver: None,
            scan_status: String::new(),
            scan_busy: false,
            status: preference_error
                .unwrap_or_else(|| "Choose a music folder to get started.".into()),
            textures: BTreeMap::new(),
            texture_usage: VecDeque::new(),
            artwork_receiver,
            artwork_pending: HashSet::new(),
            artwork_failed: HashSet::new(),
            artwork_sender,
            media_key_receiver,
            media_key_sender,
            #[cfg(target_os = "windows")]
            media_controls: None,
            #[cfg(target_os = "windows")]
            media_controls_initialized: false,
            #[cfg(target_os = "windows")]
            published_media_state: None,
        };
        if !app.folders.is_empty() {
            app.start_scan();
        }
        app
    }

    fn collection_limit(&mut self, total: usize) -> usize {
        if self.loaded_collection_page != Some(self.page)
            || self.loaded_collection_query != self.query
        {
            self.loaded_collection_page = Some(self.page);
            self.loaded_collection_query.clone_from(&self.query);
            self.loaded_collection_items = 50;
        }
        self.loaded_collection_items.min(total)
    }

    fn load_next_collection_batch(
        &mut self,
        offset_y: f32,
        viewport_height: f32,
        total_items: usize,
        loaded_items: usize,
        batch_size: usize,
        loaded_extent: f32,
        near_end_distance: f32,
    ) {
        let visible_end = offset_y + viewport_height;
        if loaded_items < total_items && visible_end >= loaded_extent - near_end_distance {
            self.loaded_collection_items = self
                .loaded_collection_items
                .saturating_add(batch_size)
                .min(total_items);
        }
    }

    #[cfg(target_os = "windows")]
    fn initialize_media_controls(&mut self, frame: &eframe::Frame, ctx: &egui::Context) {
        if self.media_controls_initialized {
            return;
        }
        self.media_controls_initialized = true;

        use raw_window_handle::{HasWindowHandle, RawWindowHandle};

        let hwnd = match frame.window_handle() {
            Ok(handle) => match handle.as_raw() {
                RawWindowHandle::Win32(handle) => Some(handle.hwnd.get() as *mut std::ffi::c_void),
                _ => None,
            },
            Err(error) => {
                self.status = format!("Could not initialize Windows media controls: {error}");
                return;
            }
        };
        let Some(hwnd) = hwnd else {
            self.status = "Windows media controls require a Win32 window handle.".into();
            return;
        };

        let config = PlatformConfig {
            dbus_name: "lilt_player",
            display_name: APP_NAME,
            hwnd: Some(hwnd),
        };
        let mut controls = match MediaControls::new(config) {
            Ok(controls) => controls,
            Err(error) => {
                self.status = format!("Could not initialize Windows media controls: {error:?}");
                return;
            }
        };
        let sender = self.media_key_sender.clone();
        let repaint_context = ctx.clone();
        if let Err(error) = controls.attach(move |event| {
            let command = match event {
                MediaControlEvent::Toggle => Some(MediaKeyCommand::Toggle),
                MediaControlEvent::Play => Some(MediaKeyCommand::Play),
                MediaControlEvent::Pause => Some(MediaKeyCommand::Pause),
                MediaControlEvent::Next => Some(MediaKeyCommand::Next),
                MediaControlEvent::Previous => Some(MediaKeyCommand::Previous),
                MediaControlEvent::Stop => Some(MediaKeyCommand::Stop),
                _ => None,
            };
            if let Some(command) = command {
                if sender.send(command).is_ok() {
                    repaint_context.request_repaint();
                }
            }
        }) {
            self.status = format!("Could not attach Windows media controls: {error:?}");
            return;
        }
        self.media_controls = Some(controls);
    }

    #[cfg(target_os = "windows")]
    fn update_media_session(&mut self) {
        let current_path = self
            .current
            .and_then(|position| self.queue.get(position))
            .and_then(|&index| self.tracks.get(index))
            .map(|track| track.path.clone());
        let playing = self
            .sink
            .as_ref()
            .is_some_and(|sink| !sink.is_paused() && !sink.empty());
        let state = (current_path.clone(), playing);
        if self.published_media_state.as_ref() == Some(&state) {
            return;
        }
        let Some(controls) = &mut self.media_controls else {
            return;
        };

        let track = current_path
            .as_ref()
            .and_then(|path| self.tracks.iter().find(|track| &track.path == path));
        if let Err(error) = controls.set_metadata(match track {
            Some(track) => souvlaki::MediaMetadata {
                title: Some(&track.title),
                artist: Some(&track.artist),
                album: Some(&track.album),
                duration: (track.duration > 0.0).then(|| Duration::from_secs_f64(track.duration)),
                ..Default::default()
            },
            None => souvlaki::MediaMetadata::default(),
        }) {
            self.status = format!("Could not update Windows media metadata: {error:?}");
            return;
        }
        let playback = if current_path.is_none() {
            souvlaki::MediaPlayback::Stopped
        } else if playing {
            souvlaki::MediaPlayback::Playing { progress: None }
        } else {
            souvlaki::MediaPlayback::Paused { progress: None }
        };
        if let Err(error) = controls.set_playback(playback) {
            self.status = format!("Could not update Windows media playback state: {error:?}");
            return;
        }
        self.published_media_state = Some(state);
    }

    fn poll_media_key_commands(&mut self) {
        while let Ok(command) = self.media_key_receiver.try_recv() {
            match command {
                MediaKeyCommand::Toggle => self.toggle_playback(),
                MediaKeyCommand::Play => {
                    if let Some(sink) = &self.sink {
                        sink.play();
                    } else if !self.queue.is_empty() {
                        self.play_queue_at(self.current.unwrap_or(0));
                    }
                }
                MediaKeyCommand::Pause => {
                    if let Some(sink) = &self.sink {
                        sink.pause();
                    }
                }
                MediaKeyCommand::Next => self.next_track(),
                MediaKeyCommand::Previous => self.previous_track(),
                MediaKeyCommand::Stop => {
                    self.sink = None;
                    self.stream = None;
                    self.current = None;
                    self.playback_offset = Duration::ZERO;
                }
            }
        }
    }

    fn queue_track_next(&mut self, index: usize) {
        if self
            .current
            .is_some_and(|position| self.queue.get(position) == Some(&index))
        {
            self.status = "That song is already playing.".into();
            return;
        }
        if let Some(position) = self.queue.iter().position(|&queued| queued == index) {
            self.queue.remove(position);
            if self.current.is_some_and(|current| position < current) {
                self.current = self.current.map(|current| current - 1);
            }
        }
        let insertion = self.current.map_or(0, |current| current + 1);
        self.queue.insert(insertion.min(self.queue.len()), index);
        self.status = format!(
            "Added {} to play next.",
            self.tracks
                .get(index)
                .map_or("song", |track| track.title.as_str())
        );
    }

    fn add_tracks_to_queue(&mut self, tracks: &[usize]) {
        let mut added = 0;
        for &index in tracks {
            if self.tracks.get(index).is_some()
                && !self.queue.contains(&index)
                && self.current.and_then(|position| self.queue.get(position)) != Some(&index)
            {
                self.queue.push(index);
                added += 1;
            }
        }
        self.status = if added == 0 {
            "Those songs are already in the queue.".into()
        } else {
            format!(
                "Added {added} {} to the queue.",
                if added == 1 { "song" } else { "songs" }
            )
        };
    }

    fn queue_tracks_next(&mut self, tracks: &[usize]) {
        for &index in tracks.iter().rev() {
            self.queue_track_next(index);
        }
        self.status = if tracks.is_empty() {
            "Those songs are already in the queue.".into()
        } else {
            "Queued the selection to play next.".into()
        };
    }

    fn create_playlist_from_tracks(&mut self, name: &str, tracks: &[usize]) {
        let paths = tracks
            .iter()
            .filter_map(|&index| self.tracks.get(index).map(|track| track.path.clone()))
            .collect::<Vec<_>>();
        if paths.is_empty() {
            self.status = "There are no available songs for that playlist.".into();
            return;
        }
        let base_name = format!("{name} Mix");
        let mut playlist_name = base_name.clone();
        let mut suffix = 2;
        while self
            .playlists
            .iter()
            .any(|playlist| playlist.name.eq_ignore_ascii_case(&playlist_name))
        {
            playlist_name = format!("{base_name} {suffix}");
            suffix += 1;
        }
        self.playlists.push(Playlist {
            name: playlist_name.clone(),
            tracks: paths,
        });
        self.save_state();
        self.status = format!("Created playlist “{playlist_name}”.");
    }

    fn show_track_context_menu(&mut self, response: &egui::Response, index: usize) {
        let Some(track) = self.tracks.get(index) else {
            return;
        };
        let title = track.title.clone();
        let path = track.path.clone();
        let album = track.album.clone();
        let artist = artist_names(&track.artist).into_iter().next();
        let liked = self.liked_songs.contains(&path);
        let disliked = self.disliked_songs.contains(&path);
        let playlist_names = self
            .playlists
            .iter()
            .map(|playlist| playlist.name.clone())
            .collect::<Vec<_>>();
        let mut play_now = false;
        let mut play_next = false;
        let mut enqueue = false;
        let mut toggle_like = false;
        let mut toggle_dislike = false;
        let mut open_album = false;
        let mut open_artist = false;
        let mut create_playlist = false;
        let mut add_to_playlist = None;
        let mut remove_from_library = false;
        let mut delete_file = false;
        response.context_menu(|ui| {
            ui.set_min_width(220.0);
            if ui.button("▶  Play now").clicked() {
                play_now = true;
                ui.close_menu();
            }
            if ui.button("⏭  Play next").clicked() {
                play_next = true;
                ui.close_menu();
            }
            if ui.button("♫  Add to queue").clicked() {
                enqueue = true;
                ui.close_menu();
            }
            ui.separator();
            if ui
                .button(if liked {
                    "♥  Unlike song"
                } else {
                    "♡  Like song"
                })
                .clicked()
            {
                toggle_like = true;
                ui.close_menu();
            }
            if ui
                .button(if disliked {
                    "−  Remove dislike"
                } else {
                    "−  Dislike song"
                })
                .clicked()
            {
                toggle_dislike = true;
                ui.close_menu();
            }
            ui.separator();
            if !is_unknown(&album) && ui.button("▣  Go to album").clicked() {
                open_album = true;
                ui.close_menu();
            }
            if artist.is_some() && ui.button("♙  Go to artist").clicked() {
                open_artist = true;
                ui.close_menu();
            }
            if ui.button("▤  Copy file path").clicked() {
                ui.ctx().copy_text(path.to_string_lossy().into_owned());
                ui.close_menu();
            }
            if !playlist_names.is_empty() {
                ui.menu_button("＋  Add to playlist", |ui| {
                    for (playlist_index, name) in playlist_names.iter().enumerate() {
                        if ui.button(name).clicked() {
                            add_to_playlist = Some(playlist_index);
                            ui.close_menu();
                        }
                    }
                });
            }
            if ui.button("＋  Create playlist from song").clicked() {
                create_playlist = true;
                ui.close_menu();
            }
            ui.separator();
            if ui.button("Remove from library").clicked() {
                remove_from_library = true;
                ui.close_menu();
            }
            if ui.button("Delete audio file…").clicked() {
                delete_file = true;
                ui.close_menu();
            }
        });
        if play_now {
            self.play_indices(vec![index], 0);
        }
        if play_next {
            self.queue_track_next(index);
        }
        if enqueue {
            self.add_tracks_to_queue(&[index]);
        }
        if toggle_like {
            self.toggle_song_reaction(index, true);
        }
        if toggle_dislike {
            self.toggle_song_reaction(index, false);
        }
        if open_album {
            self.selected_album = Some(album);
            self.page = Page::AlbumDetail;
        }
        if open_artist {
            self.selected_artist = artist;
            self.page = Page::ArtistDetail;
        }
        if create_playlist {
            self.create_playlist_from_tracks(&title, &[index]);
        }
        if let Some(playlist_index) = add_to_playlist {
            let added_to = if let Some(playlist) = self.playlists.get_mut(playlist_index) {
                if !playlist.tracks.contains(&path) {
                    playlist.tracks.push(path.clone());
                    Some(playlist.name.clone())
                } else {
                    None
                }
            } else {
                None
            };
            if let Some(playlist_name) = added_to {
                self.save_state();
                self.status = format!("Added {title} to {playlist_name}.");
            }
        }
        if remove_from_library {
            self.excluded_tracks.insert(path.clone());
            self.recommendations_dirty = true;
            self.save_state();
            self.status = format!("Removed {title} from the library. The audio file was kept.");
        }
        if delete_file {
            self.pending_delete = Some((path, title));
        }
    }

    fn show_album_context_menu(
        &mut self,
        response: &egui::Response,
        album: &str,
        tracks: &[usize],
    ) {
        let album = album.to_string();
        let tracks = tracks.to_vec();
        let saved = self.liked_albums.contains(&album.to_lowercase());
        let mut play = false;
        let mut enqueue = false;
        let mut toggle_saved = false;
        let mut create_playlist = false;
        let mut open = false;
        response.context_menu(|ui| {
            ui.set_min_width(220.0);
            if ui.button("▶  Play album").clicked() {
                play = true;
                ui.close_menu();
            }
            if ui.button("⏭  Play album next").clicked() {
                enqueue = true;
                ui.close_menu();
            }
            if ui
                .button(if saved {
                    "♥  Remove saved album"
                } else {
                    "♡  Save album"
                })
                .clicked()
            {
                toggle_saved = true;
                ui.close_menu();
            }
            if ui.button("▣  Open album page").clicked() {
                open = true;
                ui.close_menu();
            }
            if ui.button("＋  Create playlist from album").clicked() {
                create_playlist = true;
                ui.close_menu();
            }
        });
        if play {
            self.play_all_indices(tracks.clone());
        }
        if enqueue {
            self.queue_tracks_next(&tracks);
        }
        if toggle_saved {
            self.toggle_album_like(&album);
        }
        if open {
            self.selected_album = Some(album.clone());
            self.page = Page::AlbumDetail;
        }
        if create_playlist {
            self.create_playlist_from_tracks(&album, &tracks);
        }
    }

    fn show_artist_context_menu(
        &mut self,
        response: &egui::Response,
        artist: &str,
        tracks: &[usize],
    ) {
        let artist = artist.to_string();
        let tracks = tracks.to_vec();
        let followed = self.liked_artists.contains(&artist.to_lowercase());
        let mut play = false;
        let mut enqueue = false;
        let mut toggle_follow = false;
        let mut create_playlist = false;
        let mut open = false;
        response.context_menu(|ui| {
            ui.set_min_width(220.0);
            if ui.button("▶  Play artist").clicked() {
                play = true;
                ui.close_menu();
            }
            if ui.button("⏭  Play artist next").clicked() {
                enqueue = true;
                ui.close_menu();
            }
            if ui
                .button(if followed {
                    "♥  Unfollow artist"
                } else {
                    "♡  Follow artist"
                })
                .clicked()
            {
                toggle_follow = true;
                ui.close_menu();
            }
            if ui.button("♙  Open artist page").clicked() {
                open = true;
                ui.close_menu();
            }
            if ui.button("＋  Create playlist from artist").clicked() {
                create_playlist = true;
                ui.close_menu();
            }
        });
        if play {
            self.play_all_indices(tracks.clone());
        }
        if enqueue {
            self.queue_tracks_next(&tracks);
        }
        if toggle_follow {
            self.toggle_artist_like(&artist);
        }
        if open {
            self.selected_artist = Some(artist.clone());
            self.page = Page::ArtistDetail;
        }
        if create_playlist {
            self.create_playlist_from_tracks(&artist, &tracks);
        }
    }

    fn show_home_card_context_menu(&mut self, response: &egui::Response, card: &HomeCard) {
        let title = card.title.clone();
        let tracks = card.tracks.clone();
        let start = card.start;
        let mut play = false;
        let mut enqueue = false;
        let mut create_playlist = false;
        response.context_menu(|ui| {
            ui.set_min_width(220.0);
            if ui.button("▶  Play from here").clicked() {
                play = true;
                ui.close_menu();
            }
            if ui.button("♫  Add mix to queue").clicked() {
                enqueue = true;
                ui.close_menu();
            }
            if ui.button("＋  Create playlist from mix").clicked() {
                create_playlist = true;
                ui.close_menu();
            }
        });
        if play {
            self.play_indices(tracks.clone(), start);
        }
        if enqueue {
            self.add_tracks_to_queue(&tracks);
        }
        if create_playlist {
            self.create_playlist_from_tracks(&title, &tracks);
        }
    }

    fn start_scan(&mut self) {
        if self.scan_busy {
            return;
        }
        let folders = self.folders.clone();
        let (sender, receiver) = mpsc::channel();
        self.scan_receiver = Some(receiver);
        self.scan_busy = true;
        self.scan_status = "Scanning folders…".into();
        self.status = "Scanning music folders…".into();
        thread::spawn(move || {
            let result = scan_folders(&folders);
            let _ = sender.send(result);
        });
    }

    fn poll_scan(&mut self) {
        let Some(receiver) = &self.scan_receiver else {
            return;
        };
        match receiver.try_recv() {
            Ok(result) => {
                let queued_paths = self
                    .queue
                    .iter()
                    .filter_map(|&index| self.tracks.get(index).map(|track| track.path.clone()))
                    .collect::<Vec<_>>();
                let current_path = self
                    .current
                    .and_then(|position| self.queue.get(position))
                    .and_then(|&index| self.tracks.get(index))
                    .map(|track| track.path.clone());
                self.scan_busy = false;
                self.scan_receiver = None;
                self.tracks = result.tracks;
                self.tracks
                    .retain(|track| !self.excluded_tracks.contains(&track.path));
                self.latest_tracks = (0..self.tracks.len()).collect();
                self.latest_tracks
                    .sort_by_key(|&index| std::cmp::Reverse(self.tracks[index].added));
                self.home_albums = self.albums(&(0..self.tracks.len()).collect::<Vec<_>>());
                self.home_albums.sort_by_key(|(_, tracks)| {
                    std::cmp::Reverse(
                        tracks
                            .iter()
                            .map(|&index| self.tracks[index].added)
                            .max()
                            .unwrap_or_default(),
                    )
                });
                let track_indices = self
                    .tracks
                    .iter()
                    .enumerate()
                    .map(|(index, track)| (track.path.clone(), index))
                    .collect::<HashMap<_, _>>();
                self.queue = queued_paths
                    .iter()
                    .filter_map(|path| track_indices.get(path).copied())
                    .collect();
                self.current = current_path.and_then(|path| {
                    self.queue
                        .iter()
                        .position(|&index| self.tracks[index].path == path)
                });
                if self.current.is_none() {
                    self.sink = None;
                    self.stream = None;
                }
                self.recommendations_dirty = true;
                self.textures.clear();
                self.texture_usage.clear();
                self.artwork_pending.clear();
                self.artwork_failed.clear();
                self.scan_status = format!("{} songs", self.tracks.len());
                self.status = if self.tracks.is_empty() {
                    "No tracks found in the selected folders.".into()
                } else {
                    format!("Ready · {} songs", self.tracks.len())
                };
                if let Some(error) = result.error {
                    self.status = error;
                }
            }
            Err(TryRecvError::Disconnected) => {
                self.scan_busy = false;
                self.scan_receiver = None;
                self.scan_status = "Library scan stopped unexpectedly.".into();
            }
            Err(TryRecvError::Empty) => {}
        }
    }

    fn poll_artwork(&mut self, context: &egui::Context) {
        loop {
            match self.artwork_receiver.try_recv() {
                Ok(result) => {
                    self.artwork_pending.remove(&result.path);
                    if let Some(rgba) = result.rgba {
                        let image = egui::ColorImage::from_rgba_unmultiplied(result.size, &rgba);
                        let texture = context.load_texture(
                            result.path.to_string_lossy(),
                            image,
                            egui::TextureOptions::LINEAR,
                        );
                        while self.textures.len() >= 256 {
                            let Some(evicted) = self.texture_usage.pop_front() else {
                                self.textures.clear();
                                break;
                            };
                            self.textures.remove(&evicted);
                        }
                        self.texture_usage.push_back(result.path.clone());
                        self.textures.insert(result.path, texture);
                    } else {
                        self.artwork_failed.insert(result.path);
                    }
                }
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }
    }

    fn add_folder(&mut self) {
        if let Some(folder) = rfd::FileDialog::new().pick_folder() {
            if !self.folders.contains(&folder) {
                self.folders.push(folder);
                self.save_state();
            }
            self.start_scan();
        }
    }

    fn save_state(&mut self) {
        let preferences = self.preferences_snapshot();
        if let Err(error) = save_preferences(&preferences) {
            self.status = error;
        }
    }

    fn delete_audio_file(&mut self, path: &Path) {
        let Some(track_index) = self.tracks.iter().position(|track| track.path == path) else {
            self.status = "The selected track is no longer in the library.".into();
            return;
        };
        let title = self.tracks[track_index].title.clone();
        let current_path = self
            .current
            .and_then(|position| self.queue.get(position))
            .and_then(|&index| self.tracks.get(index))
            .map(|track| track.path.clone());
        let deleting_current = current_path.as_deref() == Some(path);
        let previous_position = self.current;
        if deleting_current {
            self.sink = None;
            self.stream = None;
            self.current = None;
        }
        if let Err(error) = fs::remove_file(path) {
            if deleting_current {
                if let Some(position) = previous_position {
                    self.play_queue_at(position);
                }
            }
            self.status = format!("Could not delete {}: {error}", path.display());
            return;
        }
        self.liked_songs.remove(path);
        self.disliked_songs.remove(path);
        self.excluded_tracks.remove(path);
        self.history.retain(|entry| entry != path);
        for playlist in &mut self.playlists {
            playlist.tracks.retain(|entry| entry != path);
        }
        if deleting_current {
            self.queue.retain(|&index| index != track_index);
            self.current = None;
        } else {
            let removed_before_current = self.current.map_or(0, |position| {
                self.queue
                    .iter()
                    .take(position)
                    .filter(|&&index| index == track_index)
                    .count()
            });
            self.queue.retain(|&index| index != track_index);
            if let Some(position) = self.current {
                self.current = Some(position.saturating_sub(removed_before_current));
            }
        }
        self.save_state();
        self.start_scan();
        self.status = format!("Permanently deleted {title}.");
    }

    fn preferences_snapshot(&self) -> Preferences {
        Preferences {
            folders: self.folders.clone(),
            liked_songs: self.liked_songs.clone(),
            disliked_songs: self.disliked_songs.clone(),
            liked_artists: self.liked_artists.clone(),
            liked_albums: self.liked_albums.clone(),
            playlists: self.playlists.clone(),
            history: self.history.clone(),
            excluded_tracks: self.excluded_tracks.clone(),
            view_mode: self.view_mode,
            album_view_mode: self.album_view_mode,
            artist_view_mode: self.artist_view_mode,
            accent: self.accent.clone(),
            shuffle: self.shuffle,
            repeat: self.repeat,
            discovery: self.discovery,
            prefer_unplayed: self.prefer_unplayed,
            recommendation_variety: self.recommendation_variety,
            recommendation_size: self.recommendation_size,
            ui_scale: self.ui_scale,
            volume: self.volume,
        }
    }

    fn toggle_song_reaction(&mut self, index: usize, like: bool) {
        let Some(path) = self.tracks.get(index).map(|track| track.path.clone()) else {
            return;
        };
        let (target, other) = if like {
            (&mut self.liked_songs, &mut self.disliked_songs)
        } else {
            (&mut self.disliked_songs, &mut self.liked_songs)
        };
        if !target.remove(&path) {
            target.insert(path.clone());
        }
        other.remove(&path);
        self.recommendations_dirty = true;
        self.save_state();
    }

    fn toggle_artist_like(&mut self, artist: &str) {
        let key = artist.to_lowercase();
        if !self.liked_artists.remove(&key) {
            self.liked_artists.insert(key);
        }
        self.recommendations_dirty = true;
        self.save_state();
    }

    fn toggle_album_like(&mut self, album: &str) {
        let key = album.to_lowercase();
        if !self.liked_albums.remove(&key) {
            self.liked_albums.insert(key);
        }
        self.recommendations_dirty = true;
        self.save_state();
    }

    fn indices_for_paths(&self, paths: &[PathBuf]) -> Vec<usize> {
        let lookup = self
            .tracks
            .iter()
            .enumerate()
            .map(|(index, track)| (track.path.as_path(), index))
            .collect::<HashMap<_, _>>();
        paths
            .iter()
            .filter(|path| !self.excluded_tracks.contains(*path))
            .filter_map(|path| lookup.get(path.as_path()).copied())
            .collect()
    }

    fn page_track_indices(&self) -> Vec<usize> {
        let mut indices = match self.page {
            Page::LikedSongs => self
                .tracks
                .iter()
                .enumerate()
                .filter(|(_, track)| self.liked_songs.contains(&track.path))
                .map(|(index, _)| index)
                .collect(),
            Page::Disliked => self
                .tracks
                .iter()
                .enumerate()
                .filter(|(_, track)| self.disliked_songs.contains(&track.path))
                .map(|(index, _)| index)
                .collect(),
            Page::History => self.indices_for_paths(&self.history),
            Page::Playlists => self
                .selected_playlist
                .and_then(|playlist| self.playlists.get(playlist))
                .map(|playlist| self.indices_for_paths(&playlist.tracks))
                .unwrap_or_default(),
            Page::LikedAlbums => {
                let albums = self.albums(&(0..self.tracks.len()).collect::<Vec<_>>());
                albums
                    .into_iter()
                    .filter(|(album, _)| self.liked_albums.contains(&album.to_lowercase()))
                    .flat_map(|(_, tracks)| tracks)
                    .collect()
            }
            Page::LikedArtists => {
                let artists = self.artists(&(0..self.tracks.len()).collect::<Vec<_>>());
                artists
                    .into_iter()
                    .filter(|(artist, _)| self.liked_artists.contains(&artist.to_lowercase()))
                    .flat_map(|(_, tracks)| tracks)
                    .collect()
            }
            Page::AlbumDetail => {
                let selected = self.selected_album.as_deref().unwrap_or_default();
                self.tracks
                    .iter()
                    .enumerate()
                    .filter(|(_, track)| {
                        track.album.eq_ignore_ascii_case(selected)
                            && !self.excluded_tracks.contains(&track.path)
                    })
                    .map(|(index, _)| index)
                    .collect()
            }
            Page::ArtistDetail => {
                let selected = self.selected_artist.as_deref().unwrap_or_default();
                self.tracks
                    .iter()
                    .enumerate()
                    .filter(|(_, track)| {
                        !self.excluded_tracks.contains(&track.path)
                            && artist_names(&track.artist)
                                .iter()
                                .any(|artist| artist.eq_ignore_ascii_case(selected))
                    })
                    .map(|(index, _)| index)
                    .collect()
            }
            _ => self.filtered_track_indices(),
        };
        if !self.query.trim().is_empty() {
            let query = self.query.trim().to_lowercase();
            indices.retain(|&index| self.tracks[index].search_text.contains(&query));
        }
        indices.retain(|&index| !self.excluded_tracks.contains(&self.tracks[index].path));
        indices
    }

    fn filtered_track_indices(&self) -> Vec<usize> {
        let query = self.query.trim().to_lowercase();
        self.tracks
            .iter()
            .enumerate()
            .filter(|(_, track)| {
                !self.excluded_tracks.contains(&track.path)
                    && (query.is_empty() || track.search_text.contains(&query))
            })
            .map(|(index, _)| index)
            .collect()
    }

    fn play_queue_at(&mut self, queue_position: usize) {
        let Some(&track_index) = self.queue.get(queue_position) else {
            return;
        };
        let Some(track) = self.tracks.get(track_index) else {
            return;
        };
        let track_path = track.path.clone();
        let track_title = track.title.clone();
        let track_artist = display_credit(&track.artist);
        match read_lyrics(&track_path) {
            Ok(lyrics) => {
                self.lyrics = lyrics;
                self.lyrics_error = None;
            }
            Err(error) => {
                self.lyrics = None;
                self.lyrics_error = Some(error);
            }
        }
        let file = match File::open(&track_path) {
            Ok(file) => file,
            Err(error) => {
                self.status = format!("Could not open {}: {error}", track_path.display());
                return;
            }
        };
        let (stream, handle) = match OutputStream::try_default() {
            Ok(output) => output,
            Err(error) => {
                self.status = format!("Audio output is unavailable: {error}");
                return;
            }
        };
        let sink = match Sink::try_new(&handle) {
            Ok(sink) => sink,
            Err(error) => {
                self.status = format!("Could not initialize audio playback: {error}");
                return;
            }
        };
        let decoder = match Decoder::new(BufReader::new(file)) {
            Ok(decoder) => decoder,
            Err(error) => {
                self.status = format!("Could not decode {}: {error}", track_path.display());
                return;
            }
        };
        sink.set_volume(self.volume);
        sink.append(decoder);
        self.sink = Some(sink);
        self.stream = Some(stream);
        self.current = Some(queue_position);
        self.playback_offset = Duration::ZERO;
        self.history.retain(|path| path != &track_path);
        self.history.insert(0, track_path);
        self.history.truncate(500);
        self.recommendations_dirty = true;
        self.save_state();
        self.status = format!("Playing {} — {}", track_title, track_artist);
    }

    fn seek_current_to(&mut self, position: Duration) {
        let Some(queue_position) = self.current else {
            return;
        };
        let Some(track_index) = self.queue.get(queue_position).copied() else {
            return;
        };
        let Some(track) = self.tracks.get(track_index) else {
            return;
        };
        let path = track.path.clone();
        let duration = Duration::from_secs_f64(track.duration.max(0.0));
        let position = position.min(duration);
        let was_paused = self.sink.as_ref().is_some_and(Sink::is_paused);

        let file = match File::open(&path) {
            Ok(file) => file,
            Err(error) => {
                self.status = format!("Could not reopen {} to seek: {error}", path.display());
                return;
            }
        };
        let mut decoder = match Decoder::new(BufReader::new(file)) {
            Ok(decoder) => decoder,
            Err(error) => {
                self.status = format!("Could not decode {} to seek: {error}", path.display());
                return;
            }
        };
        let decoder_seeks = decoder.try_seek(position).is_ok();

        let (stream, handle) = match OutputStream::try_default() {
            Ok(output) => output,
            Err(error) => {
                self.status = format!("Audio output is unavailable: {error}");
                return;
            }
        };
        let sink = match Sink::try_new(&handle) {
            Ok(sink) => sink,
            Err(error) => {
                self.status = format!("Could not initialize audio playback: {error}");
                return;
            }
        };
        sink.set_volume(self.volume);
        if decoder_seeks {
            sink.append(decoder);
        } else {
            let file = match File::open(&path) {
                Ok(file) => file,
                Err(error) => {
                    self.status = format!(
                        "Could not reopen {} for fallback seeking: {error}",
                        path.display()
                    );
                    return;
                }
            };
            let decoder = match Decoder::new(BufReader::new(file)) {
                Ok(decoder) => decoder,
                Err(error) => {
                    self.status = format!("Could not decode {} to seek: {error}", path.display());
                    return;
                }
            };
            sink.append(decoder.skip_duration(position));
        }
        if was_paused {
            sink.pause();
        }
        self.sink = Some(sink);
        self.stream = Some(stream);
        self.playback_offset = position;
        self.status = format!("Playing from {}", format_time(position.as_secs_f64()));
    }

    fn play_indices(&mut self, tracks: Vec<usize>, start: usize) {
        self.play_indices_with_shuffle(tracks, start, false);
    }

    fn play_all_indices(&mut self, tracks: Vec<usize>) {
        self.play_indices_with_shuffle(tracks, 0, true);
    }

    fn play_indices_with_shuffle(&mut self, tracks: Vec<usize>, start: usize, shuffle_all: bool) {
        if tracks.is_empty() {
            self.status = "There are no playable songs in this selection.".into();
            return;
        }
        self.queue = tracks;
        let start = start.min(self.queue.len() - 1);
        if self.shuffle {
            if shuffle_all {
                shuffle_slice(&mut self.queue);
            } else {
                let selected = self.queue.remove(start);
                shuffle_slice(&mut self.queue);
                self.queue.insert(0, selected);
            }
            self.play_queue_at(0);
        } else {
            self.play_queue_at(start);
        }
    }

    fn toggle_playback(&mut self) {
        if let Some(sink) = &self.sink {
            if sink.is_paused() {
                sink.play();
            } else {
                sink.pause();
            }
        } else if !self.queue.is_empty() {
            self.play_queue_at(self.current.unwrap_or(0));
        } else {
            self.play_indices(self.filtered_track_indices(), 0);
        }
    }

    fn next_track(&mut self) {
        if self.repeat == RepeatMode::One {
            if let Some(current) = self.current {
                self.play_queue_at(current);
                return;
            }
        }
        let next = self.current.map_or(0, |current| current + 1);
        if next < self.queue.len() {
            self.play_queue_at(next);
        } else if self.repeat == RepeatMode::All && !self.queue.is_empty() {
            self.play_queue_at(0);
        }
    }

    fn previous_track(&mut self) {
        if let Some(current) = self.current {
            if current > 0 {
                self.play_queue_at(current - 1);
            } else {
                self.play_queue_at(0);
            }
        }
    }

    fn swap_queue_positions(&mut self, first: usize, second: usize) {
        self.queue.swap(first, second);
        self.current = self.current.map(|current| {
            if current == first {
                second
            } else if current == second {
                first
            } else {
                current
            }
        });
    }

    fn track_row(&mut self, ui: &mut egui::Ui, index: usize) {
        let cover = self.artwork_texture(ui.ctx(), index);
        let Some(track) = self.tracks.get(index) else {
            return;
        };
        let track_path = track.path.clone();
        let title = track.title.clone();
        let artist = display_credit(&track.artist);
        let album = display_metadata(&track.album);
        let duration = format_time(track.duration);
        let genre = display_metadata(&track.genre);
        let year = track.year;
        let active = self.current.is_some_and(|position| {
            self.queue
                .get(position)
                .is_some_and(|&queued| queued == index)
        });
        let liked = self.liked_songs.contains(&track.path);
        let disliked = self.disliked_songs.contains(&track.path);
        let mut play = false;
        let mut queue = false;
        let mut like = false;
        let mut dislike = false;
        let mut remove = false;
        let mut delete_file = false;
        let mut add_to_playlist = false;
        let row_height = if self.view_mode == ViewMode::Details {
            96.0
        } else {
            84.0
        };
        let row_width = ui.available_width().max(420.0);
        let row_id = ui.make_persistent_id(("track-row", index));
        let row_rect = ui.allocate_space(Vec2::new(row_width, row_height)).1;
        let row_response = ui.interact(row_rect, row_id, egui::Sense::click());
        let hovered = row_response.hovered();
        let hover_mix = ui.ctx().animate_value_with_time(
            row_id.with("hover"),
            if hovered { 1.0 } else { 0.0 },
            0.16,
        );
        let base_fill = if active {
            Color32::from_rgb(24, 35, 78)
        } else {
            Color32::TRANSPARENT
        };
        let fill = base_fill.linear_multiply(1.0 - hover_mix)
            + Color32::from_rgb(24, 24, 24).linear_multiply(hover_mix);
        let accent = accent_color(&self.accent);
        let stroke = if active || hovered {
            egui::Stroke::new(
                1.0_f32,
                accent.linear_multiply(if active { 0.65 } else { 0.25 }),
            )
        } else {
            egui::Stroke::NONE
        };
        let mut row_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(row_rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        egui::Frame::NONE
            .fill(fill)
            .stroke(stroke)
            .inner_margin(egui::Margin::symmetric(12, 7))
            .corner_radius(egui::CornerRadius::same(9))
            .show(&mut row_ui, |ui| {
                ui.horizontal(|ui| {
                    let art_size = if self.view_mode == ViewMode::Grid {
                        56.0
                    } else {
                        56.0
                    };
                    if let Some(cover) = &cover {
                        ui.image((cover.id(), Vec2::splat(art_size)));
                    } else {
                        ui.add_sized(
                            [art_size, art_size],
                            egui::Button::new(RichText::new("♫").color(accent)),
                        );
                    }
                    ui.label(RichText::new(if active { "▶" } else { "♫" }).color(accent));
                    let action_width =
                        if self.page == Page::Playlists && self.selected_playlist.is_some() {
                            430.0
                        } else {
                            390.0
                        };
                    let text_width = (ui.available_width() - action_width - 20.0).max(100.0);
                    ui.allocate_ui_with_layout(
                        Vec2::new(text_width, row_height - 18.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.add_sized(
                                [text_width, 24.0],
                                egui::Label::new(
                                    RichText::new(&title)
                                        .strong()
                                        .color(Color32::WHITE)
                                        .size(15.0),
                                )
                                .truncate(),
                            );
                            ui.add_sized(
                                [text_width, 20.0],
                                egui::Label::new(
                                    RichText::new(format!("{artist}  ·  {album}"))
                                        .small()
                                        .color(Color32::from_rgb(170, 172, 184)),
                                )
                                .truncate(),
                            );
                            if self.view_mode == ViewMode::Details {
                                ui.add_sized(
                                    [text_width, 18.0],
                                    egui::Label::new(
                                        RichText::new(format!(
                                            "{genre}  ·  {}",
                                            if year == 0 {
                                                "Year unknown".into()
                                            } else {
                                                year.to_string()
                                            }
                                        ))
                                        .small()
                                        .color(Color32::GRAY),
                                    )
                                    .truncate(),
                                );
                            }
                        },
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let icon_button = |label: &str| {
                            egui::Button::new(label)
                                .min_size(Vec2::new(40.0, 40.0))
                                .frame(false)
                        };
                        if ui
                            .add(icon_button("⋯"))
                            .on_hover_text("Add to queue")
                            .clicked()
                        {
                            queue = true;
                        }
                        if self.page == Page::Playlists
                            && self.selected_playlist.is_some()
                            && ui
                                .add(icon_button("＋"))
                                .on_hover_text("Add to playlist")
                                .clicked()
                        {
                            add_to_playlist = true;
                        }
                        if ui
                            .add(icon_button("×"))
                            .on_hover_text("Remove from library; keeps the file")
                            .clicked()
                        {
                            remove = true;
                        }
                        if ui
                            .add(icon_button("🗑"))
                            .on_hover_text("Delete audio file permanently")
                            .clicked()
                        {
                            delete_file = true;
                        }
                        if ui
                            .selectable_label(disliked, RichText::new("−").size(17.0))
                            .on_hover_text("Dislike")
                            .clicked()
                        {
                            dislike = true;
                        }
                        if ui
                            .selectable_label(
                                liked,
                                RichText::new("♥").size(16.0).color(if liked {
                                    accent
                                } else {
                                    Color32::GRAY
                                }),
                            )
                            .on_hover_text("Like")
                            .clicked()
                        {
                            like = true;
                        }
                        ui.label(RichText::new(duration).small().color(Color32::GRAY));
                        if ui.add(icon_button("▶")).on_hover_text("Play").clicked() {
                            play = true;
                        }
                    });
                });
            });
        if play {
            let visible = self.filtered_track_indices();
            let start = visible.iter().position(|&item| item == index).unwrap_or(0);
            self.play_indices(visible, start);
        }
        self.show_track_context_menu(&row_response, index);
        if queue && !self.queue.contains(&index) {
            self.queue.push(index);
            self.status = format!("Added {title} to the queue.");
        }
        if like {
            self.toggle_song_reaction(index, true);
        }
        if dislike {
            self.toggle_song_reaction(index, false);
        }
        if remove {
            self.excluded_tracks.insert(track_path.clone());
            self.recommendations_dirty = true;
            self.save_state();
            self.status = format!("Removed {title} from the library. The audio file was kept.");
        }
        if delete_file {
            self.pending_delete = Some((track_path.clone(), title.clone()));
        }
        if add_to_playlist {
            if let Some(playlist) = self
                .selected_playlist
                .and_then(|selected| self.playlists.get_mut(selected))
            {
                if !playlist.tracks.contains(&track_path) {
                    playlist.tracks.push(track_path);
                    self.save_state();
                }
            }
        }
    }

    fn albums(&self, indices: &[usize]) -> Vec<(String, Vec<usize>)> {
        let mut albums: BTreeMap<String, (String, Vec<usize>)> = BTreeMap::new();
        for &index in indices {
            let track = &self.tracks[index];
            if self.excluded_tracks.contains(&track.path) || is_unknown(&track.album) {
                continue;
            }
            let key = track.album.trim().to_lowercase();
            let entry = albums
                .entry(key)
                .or_insert_with(|| (track.album.trim().to_string(), Vec::new()));
            entry.1.push(index);
        }
        albums.into_values().collect()
    }

    fn artists(&self, indices: &[usize]) -> Vec<(String, Vec<usize>)> {
        let mut artists: BTreeMap<String, (String, Vec<usize>)> = BTreeMap::new();
        for &index in indices {
            if self.excluded_tracks.contains(&self.tracks[index].path) {
                continue;
            }
            for name in artist_names(&self.tracks[index].artist) {
                let entry = artists
                    .entry(name.to_lowercase())
                    .or_insert_with(|| (name, Vec::new()));
                if !entry.1.contains(&index) {
                    entry.1.push(index);
                }
            }
        }
        artists.into_values().collect()
    }

    fn show_library(&mut self, ui: &mut egui::Ui) {
        let page_fade = ui.ctx().animate_bool_with_time(
            egui::Id::new(("page-fade", self.page as u8)),
            true,
            0.22,
        );
        ui.set_opacity(page_fade);
        let indices = self.page_track_indices();
        let title = match self.page {
            Page::Home => "Home".to_string(),
            Page::Songs => "Quick picks".to_string(),
            Page::Albums => "Albums".to_string(),
            Page::Artists => "Artists".to_string(),
            Page::ArtistDetail => self.selected_artist.clone().unwrap_or("Artist".into()),
            Page::AlbumDetail => self.selected_album.clone().unwrap_or("Album".into()),
            Page::LikedSongs => "Liked music".to_string(),
            Page::LikedArtists => "Liked artists".to_string(),
            Page::LikedAlbums => "Saved albums".to_string(),
            Page::Playlists => "Playlists".to_string(),
            Page::History => "Listening history".to_string(),
            Page::Disliked => "Disliked".to_string(),
            Page::Lyrics => "Lyrics".to_string(),
            Page::Settings => "Settings".to_string(),
        };
        if self.page != Page::Home {
            let mut leave_detail = false;
            ui.horizontal(|ui| {
                if matches!(self.page, Page::ArtistDetail | Page::AlbumDetail)
                    && ui.button("← Back").clicked()
                {
                    leave_detail = true;
                }
                ui.heading(RichText::new(&title).size(28.0).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    self.show_view_mode_selector(ui);
                    if self.page == Page::History && ui.small_button("Clear history").clicked() {
                        self.history.clear();
                        self.save_state();
                    }
                    if self.page != Page::Settings && self.page != Page::Playlists {
                        ui.label(
                            RichText::new(format!("{} tracks", indices.len()))
                                .small()
                                .color(Color32::GRAY),
                        );
                    }
                });
            });
            if leave_detail {
                self.page = if self.page == Page::ArtistDetail {
                    Page::Artists
                } else {
                    Page::Albums
                };
            }
            ui.add_space(12.0);
        }
        match self.page {
            Page::Settings => self.show_settings(ui),
            Page::Lyrics => {
                if let Some(position) = self.current {
                    if let Some(track) = self
                        .queue
                        .get(position)
                        .and_then(|&index| self.tracks.get(index))
                    {
                        ui.heading(&track.title);
                        ui.label(RichText::new(display_credit(&track.artist)).color(Color32::GRAY));
                    }
                }
                if let Some(error) = &self.lyrics_error {
                    ui.colored_label(Color32::LIGHT_RED, error);
                } else if let Some(lyrics) = &self.lyrics {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        ui.label(RichText::new(lyrics).size(18.0).line_height(Some(28.0)));
                    });
                } else {
                    ui.label(
                        RichText::new("No neighboring .lrc lyrics file was found for this track.")
                            .color(Color32::GRAY),
                    );
                }
            }
            Page::Home => {
                egui::Frame::NONE
                    .inner_margin(egui::Margin::symmetric(34, 14))
                    .show(ui, |ui| self.show_home(ui));
            }
            Page::Songs | Page::LikedSongs | Page::History | Page::Disliked => {
                if self.page == Page::Songs {
                    self.show_quick_picks(ui, &indices);
                } else {
                    self.show_track_collection(ui, &indices, "tracks");
                }
            }
            Page::Playlists if self.selected_playlist.is_none() => self.show_playlist_library(ui),
            Page::Playlists => {
                ui.horizontal(|ui| {
                    if ui.button("← Playlists").clicked() {
                        self.selected_playlist = None;
                    }
                    if ui.button("＋ Add now playing").clicked() {
                        if let Some(path) = self
                            .current
                            .and_then(|position| self.queue.get(position))
                            .and_then(|&index| self.tracks.get(index))
                            .map(|track| track.path.clone())
                        {
                            if let Some(playlist) = self
                                .selected_playlist
                                .and_then(|index| self.playlists.get_mut(index))
                            {
                                if !playlist.tracks.contains(&path) {
                                    playlist.tracks.push(path);
                                    self.save_state();
                                }
                            }
                        }
                    }
                });
                self.show_track_collection(ui, &indices, "playlist");
            }
            Page::Albums | Page::LikedAlbums => {
                self.show_album_collection(ui);
            }
            Page::Artists | Page::LikedArtists => self.show_artist_collection(ui, &indices),
            Page::ArtistDetail => self.show_entity_detail(ui, &indices, true),
            Page::AlbumDetail => self.show_entity_detail(ui, &indices, false),
        }
    }

    fn show_entity_detail(&mut self, ui: &mut egui::Ui, tracks: &[usize], is_artist: bool) {
        let title = if is_artist {
            self.selected_artist.clone().unwrap_or("Artist".into())
        } else {
            self.selected_album.clone().unwrap_or("Album".into())
        };
        let cover = tracks
            .first()
            .and_then(|&index| self.artwork_texture(ui.ctx(), index));
        let mut play_collection = false;
        ui.group(|ui| {
            ui.horizontal(|ui| {
                let (cover_rect, _) =
                    ui.allocate_exact_size(Vec2::splat(132.0), egui::Sense::hover());
                paint_cover_or_placeholder(ui, cover.as_ref(), cover_rect, tracks.len());
                ui.vertical(|ui| {
                    ui.heading(RichText::new(&title).size(27.0).strong());
                    let metadata = if is_artist {
                        format!("{} tracks", tracks.len())
                    } else {
                        let credits = merge_credits(
                            tracks
                                .iter()
                                .filter_map(|&index| self.tracks.get(index))
                                .map(|track| track.artist.as_str()),
                        );
                        format!("{} · {} tracks", credits.join(", "), tracks.len())
                    };
                    ui.label(RichText::new(metadata).color(Color32::GRAY));
                    if ui
                        .add_sized([144.0, 42.0], egui::Button::new("▶  Play collection"))
                        .clicked()
                    {
                        play_collection = true;
                    }
                });
            });
        });
        if play_collection {
            self.play_all_indices(tracks.to_vec());
        }
        if is_artist {
            let albums = self.albums(tracks);
            if !albums.is_empty() {
                ui.add_space(12.0);
                ui.label(RichText::new("Albums").strong().size(18.0));
                ui.horizontal_wrapped(|ui| {
                    for (album, _) in albums {
                        if ui.button(format!("▣  {album}")).clicked() {
                            self.selected_album = Some(album);
                            self.page = Page::AlbumDetail;
                        }
                    }
                });
            }
        } else {
            let artists = merge_credits(
                tracks
                    .iter()
                    .filter_map(|&index| self.tracks.get(index))
                    .map(|track| track.artist.as_str()),
            );
            if !artists.is_empty() {
                ui.add_space(12.0);
                ui.label(RichText::new("Artists").strong().size(18.0));
                ui.horizontal_wrapped(|ui| {
                    for artist in artists {
                        if ui.button(format!("♙  {artist}")).clicked() {
                            self.selected_artist = Some(artist);
                            self.page = Page::ArtistDetail;
                        }
                    }
                });
            }
        }
        ui.add_space(14.0);
        ui.label(RichText::new("Tracks").strong().size(18.0));
        self.show_track_collection(ui, tracks, "entity-detail");
    }

    fn show_home(&mut self, ui: &mut egui::Ui) {
        let recommendations = self.recommendation_indices();
        let available = self.home_track_indices();
        let available_set = available.iter().copied().collect::<HashSet<_>>();
        let mut recent = Vec::new();
        let mut seen = HashSet::new();
        for track_index in self.indices_for_paths(&self.history) {
            if available_set.contains(&track_index) && seen.insert(track_index) {
                recent.push(track_index);
            }
            if recent.len() >= 40 {
                break;
            }
        }
        if recent.is_empty() {
            recent = available.iter().copied().take(40).collect();
        }
        let recent_paths = self
            .indices_for_paths(&self.history)
            .into_iter()
            .filter(|index| recent.contains(index))
            .filter_map(|index| self.tracks.get(index).map(|track| track.path.clone()))
            .collect::<HashSet<_>>();
        let played_paths = self.history.iter().cloned().collect::<HashSet<_>>();
        let unplayed = recommendations
            .iter()
            .copied()
            .filter(|&index| {
                available_set.contains(&index) && !played_paths.contains(&self.tracks[index].path)
            })
            .take(self.recommendation_size)
            .collect::<Vec<_>>();
        let latest = self
            .latest_tracks
            .iter()
            .copied()
            .filter(|index| available_set.contains(index))
            .take(40)
            .collect::<Vec<_>>();
        let mut fresh = latest
            .iter()
            .copied()
            .filter(|&index| !recent_paths.contains(&self.tracks[index].path))
            .collect::<Vec<_>>();
        if fresh.is_empty() {
            fresh = available.iter().copied().take(40).collect();
        }
        let liked = self
            .tracks
            .iter()
            .enumerate()
            .filter(|(index, track)| {
                available_set.contains(index) && self.liked_songs.contains(&track.path)
            })
            .map(|(index, _)| index)
            .take(40)
            .collect::<Vec<_>>();
        let mut recommendations = recommendations
            .into_iter()
            .filter(|index| available_set.contains(index))
            .take(self.recommendation_size)
            .collect::<Vec<_>>();
        if recommendations.is_empty() {
            recommendations = available
                .iter()
                .copied()
                .take(self.recommendation_size)
                .collect();
        }
        let recommendation_order = recommendations
            .iter()
            .enumerate()
            .map(|(position, &index)| (index, position))
            .collect::<HashMap<_, _>>();
        let mut mixes = Vec::new();
        if !recommendations.is_empty() {
            mixes.push(HomeCard {
                title: "My Mix".into(),
                subtitle: "Made for you".into(),
                cover_index: recommendations[0],
                tracks: recommendations.clone(),
                start: 0,
            });
        }
        if !fresh.is_empty() {
            mixes.push(HomeCard {
                title: "Recently added".into(),
                subtitle: "Latest additions to your library".into(),
                cover_index: fresh[0],
                tracks: fresh.clone(),
                start: 0,
            });
        }
        if !liked.is_empty() {
            mixes.push(HomeCard {
                title: "Liked songs".into(),
                subtitle: format!("{} favorites", liked.len()),
                cover_index: liked[0],
                tracks: liked.clone(),
                start: 0,
            });
        }
        if let Some(&seed) = recommendations.first() {
            let seed_artists = artist_names(&self.tracks[seed].artist)
                .into_iter()
                .map(|artist| artist.to_lowercase())
                .collect::<HashSet<_>>();
            let mut radio = available
                .iter()
                .copied()
                .filter(|&index| {
                    let track = &self.tracks[index];
                    artist_names(&track.artist)
                        .iter()
                        .any(|artist| seed_artists.contains(&artist.to_lowercase()))
                })
                .collect::<Vec<_>>();
            radio.sort_by_key(|index| {
                recommendation_order
                    .get(index)
                    .copied()
                    .unwrap_or(usize::MAX)
            });
            radio.truncate(self.recommendation_size);
            if !radio.is_empty() {
                mixes.push(HomeCard {
                    title: format!("Radio: {}", display_credit(&self.tracks[seed].artist)),
                    subtitle: "More around your sound".into(),
                    cover_index: radio[0],
                    tracks: radio,
                    start: 0,
                });
            }
        }
        for artist in self.liked_artists.iter().take(6) {
            let mut radio = available
                .iter()
                .copied()
                .filter(|&index| {
                    artist_names(&self.tracks[index].artist)
                        .iter()
                        .any(|name| name.eq_ignore_ascii_case(artist))
                })
                .collect::<Vec<_>>();
            radio.sort_by_key(|index| {
                recommendation_order
                    .get(index)
                    .copied()
                    .unwrap_or(usize::MAX)
            });
            radio.truncate(self.recommendation_size);
            if let Some(&cover_index) = radio.first() {
                mixes.push(HomeCard {
                    title: format!("{artist} Radio"),
                    subtitle: "More from an artist you follow".into(),
                    cover_index,
                    tracks: radio,
                    start: 0,
                });
            }
        }

        let mut genre_counts = BTreeMap::<String, usize>::new();
        for track in &self.tracks {
            if self.excluded_tracks.contains(&track.path)
                || self.disliked_songs.contains(&track.path)
            {
                continue;
            }
            let genre = display_metadata(&track.genre);
            if genre != "—" {
                *genre_counts.entry(genre).or_default() += 1;
            }
        }
        let mut genres = genre_counts.into_iter().collect::<Vec<_>>();
        genres.sort_by(|left, right| {
            right
                .1
                .cmp(&left.1)
                .then_with(|| left.0.to_lowercase().cmp(&right.0.to_lowercase()))
        });
        genres.truncate(12);
        let genre_mix_cards = genres
            .iter()
            .filter_map(|(genre, _)| {
                let mut tracks = available
                    .iter()
                    .copied()
                    .filter(|&index| self.tracks[index].genre.eq_ignore_ascii_case(genre))
                    .collect::<Vec<_>>();
                tracks.sort_by_key(|index| {
                    recommendation_order
                        .get(index)
                        .copied()
                        .unwrap_or(usize::MAX)
                });
                tracks.truncate(self.recommendation_size);
                let cover_index = *tracks.first()?;
                Some(HomeCard {
                    title: format!("{genre} Mix"),
                    subtitle: format!("{} tracks from your library", tracks.len()),
                    cover_index,
                    tracks,
                    start: 0,
                })
            })
            .take(8)
            .collect::<Vec<_>>();

        let library_cards = self
            .home_albums
            .iter()
            .filter_map(|(album, album_tracks)| {
                let tracks = album_tracks
                    .iter()
                    .copied()
                    .filter(|index| available_set.contains(index))
                    .collect::<Vec<_>>();
                let &cover_index = tracks.first()?;
                let artist = merge_credits(
                    tracks
                        .iter()
                        .map(|&index| self.tracks[index].artist.as_str()),
                );
                Some(HomeCard {
                    title: album.clone(),
                    subtitle: if artist.is_empty() {
                        format!("{} tracks", tracks.len())
                    } else {
                        artist.join(", ")
                    },
                    cover_index,
                    tracks,
                    start: 0,
                })
            })
            .take(30)
            .collect::<Vec<_>>();
        let recent_cards = recent
            .iter()
            .enumerate()
            .filter_map(|(start, &index)| {
                let track = self.tracks.get(index)?;
                Some(HomeCard {
                    title: track.title.clone(),
                    subtitle: display_credit(&track.artist),
                    cover_index: index,
                    tracks: recent.clone(),
                    start,
                })
            })
            .collect::<Vec<_>>();
        let latest_cards = latest
            .iter()
            .enumerate()
            .filter_map(|(start, &index)| {
                let track = self.tracks.get(index)?;
                Some(HomeCard {
                    title: track.title.clone(),
                    subtitle: display_credit(&track.artist),
                    cover_index: index,
                    tracks: latest.clone(),
                    start,
                })
            })
            .collect::<Vec<_>>();

        egui::ScrollArea::vertical()
            .id_salt("home-feed-v2")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let page_rect = ui.max_rect();
                let band_height = 72.0;
                let gradient = [
                    Color32::from_rgb(34, 13, 23),
                    Color32::from_rgb(28, 12, 17),
                    Color32::from_rgb(20, 10, 12),
                    Color32::from_rgb(0, 0, 0),
                ];
                for (band, color) in gradient.into_iter().enumerate() {
                    ui.painter().rect_filled(
                        egui::Rect::from_min_size(
                            egui::pos2(
                                page_rect.min.x,
                                page_rect.min.y + band as f32 * band_height,
                            ),
                            Vec2::new(page_rect.width(), band_height + 1.0),
                        ),
                        0.0,
                        color,
                    );
                }
                ui.add_space(4.0);
                egui::ScrollArea::horizontal()
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let mut chips = vec!["All music".to_string()];
                            chips.extend(genres.iter().map(|(genre, _)| genre.clone()));
                            for genre in chips {
                                let selected = if genre == "All music" {
                                    self.home_genre.is_empty()
                                } else {
                                    self.home_genre.eq_ignore_ascii_case(&genre)
                                };
                                if ui
                                    .add_sized(
                                        [
                                            ui.fonts(|fonts| {
                                                fonts
                                                    .layout_no_wrap(
                                                        genre.clone(),
                                                        egui::FontId::proportional(14.0),
                                                        Color32::WHITE,
                                                    )
                                                    .size()
                                                    .x
                                                    + 30.0
                                            })
                                            .max(84.0),
                                            36.0,
                                        ],
                                        egui::Button::new(genre.clone()).fill(if selected {
                                            Color32::from_rgb(76, 43, 53)
                                        } else {
                                            Color32::from_rgb(41, 30, 35)
                                        }),
                                    )
                                    .clicked()
                                {
                                    self.home_genre = if genre == "All music" {
                                        String::new()
                                    } else {
                                        genre
                                    };
                                }
                            }
                        });
                    });
                ui.add_space(18.0);
                self.show_home_shelf(
                    ui,
                    "Listen again",
                    "YOUR RECENT ROTATION",
                    recent_cards,
                    Some(Page::History),
                );
                ui.add_space(22.0);
                self.show_home_shelf(
                    ui,
                    "Recommendations",
                    "PERSONALIZED PICKS · TUNED TO YOUR LISTENING",
                    mixes,
                    None,
                );
                ui.add_space(22.0);
                let unplayed_cards = unplayed
                    .iter()
                    .enumerate()
                    .filter_map(|(start, &index)| {
                        let track = self.tracks.get(index)?;
                        Some(HomeCard {
                            title: track.title.clone(),
                            subtitle: format!("{} · New to you", display_credit(&track.artist)),
                            cover_index: index,
                            tracks: unplayed.clone(),
                            start,
                        })
                    })
                    .collect::<Vec<_>>();
                self.show_home_shelf(
                    ui,
                    "New to you",
                    "RECOMMENDED SONGS YOU HAVEN'T PLAYED YET",
                    unplayed_cards,
                    None,
                );
                ui.add_space(22.0);
                self.show_home_shelf(
                    ui,
                    "Your genre mixes",
                    "PICK A SOUND FROM YOUR LIBRARY",
                    genre_mix_cards,
                    None,
                );
                ui.add_space(22.0);
                self.show_home_shelf(
                    ui,
                    "From your library",
                    "ALBUMS READY FOR ANOTHER LISTEN",
                    library_cards,
                    Some(Page::Albums),
                );
                ui.add_space(22.0);
                self.show_home_shelf(
                    ui,
                    "Recently added",
                    "NEW IN YOUR COLLECTION",
                    latest_cards,
                    Some(Page::Songs),
                );
            });
    }

    fn home_track_indices(&self) -> Vec<usize> {
        self.tracks
            .iter()
            .enumerate()
            .filter(|(_, track)| {
                !self.excluded_tracks.contains(&track.path)
                    && !self.disliked_songs.contains(&track.path)
                    && (self.home_genre.is_empty()
                        || track.genre.eq_ignore_ascii_case(&self.home_genre))
            })
            .map(|(index, _)| index)
            .collect()
    }

    fn show_album_collection(&mut self, ui: &mut egui::Ui) {
        let query = self.query.trim().to_lowercase();
        let album_ids = self
            .home_albums
            .iter()
            .enumerate()
            .filter(|(_, (album, tracks))| {
                (self.page != Page::LikedAlbums
                    || self.liked_albums.contains(&album.to_lowercase()))
                    && (query.is_empty()
                        || album.to_lowercase().contains(&query)
                        || tracks.iter().any(|&index| {
                            self.tracks
                                .get(index)
                                .is_some_and(|track| track.search_text.contains(&query))
                        }))
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        let loaded = self.collection_limit(album_ids.len());
        const BATCH_SIZE: usize = 50;
        let mut play_tracks = None;
        let mut save_album = None;
        let mut open_album = None;
        if self.album_view_mode == ViewMode::Grid {
            let columns = ((ui.available_width() / 220.0).floor() as usize).clamp(1, 7);
            let row_count = loaded.div_ceil(columns);
            let output = egui::ScrollArea::vertical()
                .id_salt(("album-collection-grid", self.page as u8))
                .auto_shrink([false, false])
                .show_rows(ui, 260.0, row_count, |ui, row_range| {
                    for row in row_range {
                        let row_width = ui.available_width();
                        let card_width = (row_width / columns as f32 - 10.0).max(170.0);
                        ui.horizontal(|ui| {
                            for column in 0..columns {
                                let position = row * columns + column;
                                if position >= loaded {
                                    break;
                                }
                                let Some((album, tracks)) = album_ids
                                    .get(position)
                                    .and_then(|&id| self.home_albums.get(id))
                                else {
                                    break;
                                };
                                let album = album.clone();
                                let tracks = tracks
                                    .iter()
                                    .copied()
                                    .filter(|&index| {
                                        !self.excluded_tracks.contains(&self.tracks[index].path)
                                    })
                                    .filter(|&index| {
                                        query.is_empty()
                                            || self.tracks[index].search_text.contains(&query)
                                    })
                                    .collect::<Vec<_>>();
                                let Some(&cover_index) = tracks.first() else {
                                    continue;
                                };
                                let artists = merge_credits(
                                    tracks
                                        .iter()
                                        .map(|&index| self.tracks[index].artist.as_str()),
                                );
                                let cover = self.artwork_texture(ui.ctx(), cover_index);
                                ui.allocate_ui_with_layout(
                                    Vec2::new(card_width, 252.0),
                                    egui::Layout::top_down(egui::Align::Center),
                                    |ui| {
                                        egui::Frame::group(ui.style()).show(ui, |ui| {
                                            ui.set_min_width(card_width - 18.0);
                                            ui.vertical_centered(|ui| {
                                                let (cover_rect, cover_response) = ui
                                                    .allocate_exact_size(
                                                        Vec2::splat(132.0),
                                                        egui::Sense::click(),
                                                    );
                                                paint_cover_or_placeholder(
                                                    ui,
                                                    cover.as_ref(),
                                                    cover_rect,
                                                    cover_index,
                                                );
                                                self.show_album_context_menu(
                                                    &cover_response,
                                                    &album,
                                                    &tracks,
                                                );
                                                if ui
                                                    .add_sized(
                                                        [card_width - 28.0, 24.0],
                                                        egui::Button::new(
                                                            RichText::new(&album).strong(),
                                                        )
                                                        .frame(false),
                                                    )
                                                    .clicked()
                                                {
                                                    open_album = Some(album.clone());
                                                }
                                                ui.add_sized(
                                                    [card_width - 28.0, 20.0],
                                                    egui::Label::new(
                                                        RichText::new(format!(
                                                            "{} · {} tracks",
                                                            artists.join(", "),
                                                            tracks.len()
                                                        ))
                                                        .small()
                                                        .color(Color32::GRAY),
                                                    )
                                                    .truncate(),
                                                );
                                                ui.horizontal(|ui| {
                                                    if ui.button("▶ Play").clicked() {
                                                        play_tracks = Some(tracks.clone());
                                                    }
                                                    if ui.button("Open").clicked() {
                                                        open_album = Some(album.clone());
                                                    }
                                                    let liked = self
                                                        .liked_albums
                                                        .contains(&album.to_lowercase());
                                                    if ui
                                                        .button(if liked {
                                                            "♥ Saved"
                                                        } else {
                                                            "♡ Save"
                                                        })
                                                        .clicked()
                                                    {
                                                        save_album = Some(album.clone());
                                                    }
                                                });
                                            });
                                        });
                                    },
                                );
                            }
                        });
                    }
                });
            if let Some(tracks) = play_tracks {
                self.play_all_indices(tracks);
            }
            if let Some(album) = save_album {
                self.toggle_album_like(&album);
            }
            if let Some(album) = open_album {
                self.selected_album = Some(album);
                self.page = Page::AlbumDetail;
            }
            self.load_next_collection_batch(
                output.state.offset.y,
                output.inner_rect.height(),
                album_ids.len(),
                loaded,
                BATCH_SIZE,
                row_count as f32 * 260.0,
                520.0,
            );
            if loaded < album_ids.len() {
                ui.label(
                    RichText::new(format!("Showing {loaded} of {} albums", album_ids.len()))
                        .small()
                        .color(Color32::GRAY),
                );
            }
            return;
        }
        let row_height = if self.album_view_mode == ViewMode::Details {
            144.0
        } else {
            92.0
        };
        let output = egui::ScrollArea::vertical()
            .id_salt(("album-collection", self.page as u8))
            .auto_shrink([false, false])
            .show_rows(ui, row_height, loaded, |ui, row_range| {
                for row in row_range {
                    let Some((album, tracks)) =
                        album_ids.get(row).and_then(|&id| self.home_albums.get(id))
                    else {
                        continue;
                    };
                    let album = album.clone();
                    let tracks = tracks
                        .iter()
                        .copied()
                        .filter(|&index| !self.excluded_tracks.contains(&self.tracks[index].path))
                        .filter(|&index| {
                            query.is_empty() || self.tracks[index].search_text.contains(&query)
                        })
                        .collect::<Vec<_>>();
                    let Some(&cover_index) = tracks.first() else {
                        continue;
                    };
                    let artists = merge_credits(
                        tracks
                            .iter()
                            .map(|&index| self.tracks[index].artist.as_str()),
                    );
                    let cover = self.artwork_texture(ui.ctx(), cover_index);
                    ui.allocate_ui_with_layout(
                        Vec2::new(ui.available_width(), row_height - 8.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            let cover_size = if self.album_view_mode == ViewMode::Details {
                                112.0
                            } else {
                                62.0
                            };
                            let (cover_rect, cover_response) = ui
                                .allocate_exact_size(Vec2::splat(cover_size), egui::Sense::click());
                            paint_cover_or_placeholder(ui, cover.as_ref(), cover_rect, cover_index);
                            self.show_album_context_menu(&cover_response, &album, &tracks);
                            ui.add_space(12.0);
                            ui.vertical(|ui| {
                                if ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new(&album).strong().size(19.0),
                                        )
                                        .frame(false),
                                    )
                                    .clicked()
                                {
                                    open_album = Some(album.clone());
                                }
                                ui.label(
                                    RichText::new(format!(
                                        "{} · {} tracks",
                                        if artists.is_empty() {
                                            "Various artists".into()
                                        } else {
                                            artists.join(", ")
                                        },
                                        tracks.len()
                                    ))
                                    .small()
                                    .color(Color32::GRAY),
                                );
                                ui.add_space(6.0);
                                ui.horizontal(|ui| {
                                    if ui
                                        .add_sized([100.0, 38.0], egui::Button::new("▶  Play"))
                                        .clicked()
                                    {
                                        play_tracks = Some(tracks.clone());
                                    }
                                    if ui.button("Open album").clicked() {
                                        open_album = Some(album.clone());
                                    }
                                    let liked = self.liked_albums.contains(&album.to_lowercase());
                                    if ui
                                        .add_sized(
                                            [120.0, 38.0],
                                            egui::Button::new(if liked {
                                                "♥  Saved"
                                            } else {
                                                "♡  Save album"
                                            }),
                                        )
                                        .clicked()
                                    {
                                        save_album = Some(album.clone());
                                    }
                                });
                            });
                        },
                    );
                    ui.separator();
                }
            });
        if let Some(tracks) = play_tracks {
            self.play_all_indices(tracks);
        }
        if let Some(album) = save_album {
            self.toggle_album_like(&album);
        }
        if let Some(album) = open_album {
            self.selected_album = Some(album);
            self.page = Page::AlbumDetail;
        }
        self.load_next_collection_batch(
            output.state.offset.y,
            output.inner_rect.height(),
            album_ids.len(),
            loaded,
            BATCH_SIZE,
            loaded as f32 * row_height,
            row_height * 2.0,
        );
        if loaded < album_ids.len() {
            ui.label(
                RichText::new(format!("Showing {loaded} of {} albums", album_ids.len()))
                    .small()
                    .color(Color32::GRAY),
            );
        }
    }

    fn show_artist_collection(&mut self, ui: &mut egui::Ui, indices: &[usize]) {
        let all_indices;
        let source = if self.page == Page::LikedArtists {
            all_indices = (0..self.tracks.len()).collect::<Vec<_>>();
            &all_indices
        } else {
            indices
        };
        let query = self.query.trim().to_lowercase();
        let artists = self
            .artists(source)
            .into_iter()
            .filter(|(artist, tracks)| {
                (self.page != Page::LikedArtists
                    || self.liked_artists.contains(&artist.to_lowercase()))
                    && (query.is_empty()
                        || artist.to_lowercase().contains(&query)
                        || tracks.iter().any(|&index| {
                            self.tracks
                                .get(index)
                                .is_some_and(|track| track.search_text.contains(&query))
                        }))
            })
            .collect::<Vec<_>>();
        const BATCH_SIZE: usize = 50;
        let loaded = self.collection_limit(artists.len());
        let columns = if self.artist_view_mode == ViewMode::Grid {
            ((ui.available_width() / 220.0).floor() as usize).clamp(1, 7)
        } else {
            1
        };
        let row_height = match self.artist_view_mode {
            ViewMode::Grid => 250.0,
            ViewMode::Details => 128.0,
            ViewMode::List => 82.0,
        };
        let row_count = loaded.div_ceil(columns);
        let mut play_artist = None;
        let mut follow_artist = None;
        let mut open_artist = None;
        let output = egui::ScrollArea::vertical()
            .id_salt(("artist-collection", self.page as u8))
            .auto_shrink([false, false])
            .show_rows(ui, row_height, row_count, |ui, row_range| {
                for row in row_range {
                    let row_width = ui.available_width();
                    let card_width = if columns == 1 {
                        row_width
                    } else {
                        (row_width / columns as f32 - 10.0).max(170.0)
                    };
                    ui.horizontal(|ui| {
                        for column in 0..columns {
                            let position = row * columns + column;
                            if position >= loaded {
                                break;
                            }
                            let Some((artist, tracks)) = artists.get(position) else {
                                break;
                            };
                            let cover = tracks
                                .first()
                                .and_then(|&index| self.artwork_texture(ui.ctx(), index));
                            let liked = self.liked_artists.contains(&artist.to_lowercase());
                            let albums = tracks
                                .iter()
                                .filter_map(|&index| self.tracks.get(index))
                                .filter(|track| !is_unknown(&track.album))
                                .map(|track| track.album.trim().to_lowercase())
                                .collect::<HashSet<_>>()
                                .len();
                            let tracks = tracks.clone();
                            let artist = artist.clone();

                            if self.artist_view_mode == ViewMode::Grid {
                                ui.allocate_ui_with_layout(
                                    Vec2::new(card_width, row_height - 8.0),
                                    egui::Layout::top_down(egui::Align::Center),
                                    |ui| {
                                        egui::Frame::group(ui.style()).show(ui, |ui| {
                                            ui.set_min_width(card_width - 18.0);
                                            ui.vertical_centered(|ui| {
                                                let (cover_rect, cover_response) = ui
                                                    .allocate_exact_size(
                                                        Vec2::splat(112.0),
                                                        egui::Sense::click(),
                                                    );
                                                paint_cover_or_placeholder(
                                                    ui,
                                                    cover.as_ref(),
                                                    cover_rect,
                                                    position,
                                                );
                                                self.show_artist_context_menu(
                                                    &cover_response,
                                                    &artist,
                                                    &tracks,
                                                );
                                                ui.add_space(6.0);
                                                if ui
                                                    .add_sized(
                                                        [card_width - 28.0, 24.0],
                                                        egui::Button::new(
                                                            RichText::new(&artist).strong(),
                                                        )
                                                        .frame(false),
                                                    )
                                                    .clicked()
                                                {
                                                    open_artist = Some(artist.clone());
                                                }
                                                ui.label(
                                                    RichText::new(format!(
                                                        "{albums} albums · {} tracks",
                                                        tracks.len()
                                                    ))
                                                    .small()
                                                    .color(Color32::GRAY),
                                                );
                                                ui.horizontal(|ui| {
                                                    if ui.button("Open").clicked() {
                                                        open_artist = Some(artist.clone());
                                                    }
                                                    if ui.button("▶ Play").clicked() {
                                                        play_artist =
                                                            Some((artist.clone(), tracks.clone()));
                                                    }
                                                    if ui
                                                        .selectable_label(
                                                            liked,
                                                            if liked {
                                                                "♥ Following"
                                                            } else {
                                                                "♡ Follow"
                                                            },
                                                        )
                                                        .clicked()
                                                    {
                                                        follow_artist = Some(artist.clone());
                                                    }
                                                });
                                            });
                                        });
                                    },
                                );
                            } else {
                                let row_rect =
                                    ui.allocate_space(Vec2::new(card_width, row_height - 6.0)).1;
                                let row_response = ui.interact(
                                    row_rect,
                                    ui.make_persistent_id(("artist-row", position)),
                                    egui::Sense::click(),
                                );
                                let hovered = row_response.hovered();
                                let painter = ui.painter();
                                painter.rect_filled(
                                    row_rect,
                                    egui::CornerRadius::same(6),
                                    if hovered {
                                        Color32::from_rgb(27, 27, 29)
                                    } else {
                                        Color32::from_rgb(10, 10, 10)
                                    },
                                );
                                painter.line_segment(
                                    [row_rect.left_bottom(), row_rect.right_bottom()],
                                    egui::Stroke::new(1.0_f32, Color32::from_rgb(40, 40, 42)),
                                );
                                let art_size = if self.artist_view_mode == ViewMode::Details {
                                    88.0
                                } else {
                                    52.0
                                };
                                let cover_rect = egui::Rect::from_center_size(
                                    egui::pos2(
                                        row_rect.left() + 12.0 + art_size / 2.0,
                                        row_rect.center().y,
                                    ),
                                    Vec2::splat(art_size),
                                );
                                paint_cover_or_placeholder(
                                    ui,
                                    cover.as_ref(),
                                    cover_rect,
                                    position,
                                );
                                self.show_artist_context_menu(&row_response, &artist, &tracks);
                                let follow_rect = egui::Rect::from_center_size(
                                    egui::pos2(row_rect.right() - 70.0, row_rect.center().y),
                                    Vec2::new(112.0, 38.0),
                                );
                                let play_rect = egui::Rect::from_center_size(
                                    egui::pos2(follow_rect.left() - 54.0, row_rect.center().y),
                                    Vec2::new(88.0, 38.0),
                                );
                                let text_left = cover_rect.right() + 14.0;
                                let text_painter =
                                    painter.with_clip_rect(egui::Rect::from_min_max(
                                        egui::pos2(text_left, row_rect.top()),
                                        egui::pos2(
                                            (play_rect.left() - 12.0).max(text_left),
                                            row_rect.bottom(),
                                        ),
                                    ));
                                text_painter.text(
                                    egui::pos2(text_left, row_rect.center().y + 14.0),
                                    egui::Align2::LEFT_CENTER,
                                    format!("{albums} albums · {} tracks", tracks.len()),
                                    egui::FontId::proportional(12.0),
                                    Color32::GRAY,
                                );
                                let title_rect = egui::Rect::from_min_max(
                                    egui::pos2(text_left, row_rect.top()),
                                    egui::pos2(
                                        (play_rect.left() - 12.0).max(text_left + 1.0),
                                        row_rect.center().y + 2.0,
                                    ),
                                );
                                if ui
                                    .put(
                                        title_rect,
                                        egui::Button::new(
                                            RichText::new(&artist).strong().size(18.0),
                                        )
                                        .frame(false),
                                    )
                                    .clicked()
                                {
                                    open_artist = Some(artist.clone());
                                }
                                if ui.put(play_rect, egui::Button::new("▶ Play")).clicked() {
                                    play_artist = Some((artist.clone(), tracks.clone()));
                                }
                                if ui
                                    .put(
                                        follow_rect,
                                        egui::Button::new(if liked {
                                            "♥ Following"
                                        } else {
                                            "♡ Follow"
                                        }),
                                    )
                                    .clicked()
                                {
                                    follow_artist = Some(artist.clone());
                                }
                            }
                        }
                    });
                }
            });
        if let Some((_, tracks)) = play_artist {
            self.play_all_indices(tracks);
        }
        if let Some(artist) = follow_artist {
            self.toggle_artist_like(&artist);
        }
        if let Some(artist) = open_artist {
            self.selected_artist = Some(artist);
            self.page = Page::ArtistDetail;
        }
        self.load_next_collection_batch(
            output.state.offset.y,
            output.inner_rect.height(),
            artists.len(),
            loaded,
            BATCH_SIZE,
            row_count as f32 * row_height,
            row_height * 2.0,
        );
        if loaded < artists.len() {
            ui.label(
                RichText::new(format!("Showing {loaded} of {} artists", artists.len()))
                    .small()
                    .color(Color32::GRAY),
            );
        }
    }

    fn show_view_mode_selector(&mut self, ui: &mut egui::Ui) {
        let (id, mode) = match self.page {
            Page::Albums | Page::LikedAlbums => ("album-view-mode", &mut self.album_view_mode),
            Page::Artists | Page::LikedArtists => ("artist-view-mode", &mut self.artist_view_mode),
            Page::LikedSongs | Page::History | Page::Disliked => {
                ("track-view-mode", &mut self.view_mode)
            }
            _ => return,
        };
        if view_mode_combo(ui, id, mode) {
            self.save_state();
        }
    }

    fn show_home_shelf(
        &mut self,
        ui: &mut egui::Ui,
        title: &str,
        eyebrow: &str,
        cards: Vec<HomeCard>,
        more_page: Option<Page>,
    ) {
        if cards.is_empty() {
            return;
        }
        let shelf_id = egui::Id::new(("home-shelf-v2", title));
        let mut shelf_scroll = self
            .home_shelf_offsets
            .get(title)
            .copied()
            .unwrap_or_default();
        let mut selected_page = None;
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(
                    RichText::new(eyebrow)
                        .small()
                        .strong()
                        .color(Color32::from_rgb(193, 157, 167)),
                );
                ui.heading(RichText::new(title).size(30.0).strong());
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(page) = more_page {
                    if ui.button("More").clicked() {
                        selected_page = Some(page);
                    }
                }
                if ui.button("›").clicked() {
                    shelf_scroll += 520.0;
                }
                if ui.button("‹").clicked() {
                    shelf_scroll = (shelf_scroll - 520.0).max(0.0);
                }
            });
        });
        if let Some(page) = selected_page {
            self.page = page;
            self.selected_playlist = None;
        }
        ui.add_space(14.0);
        let output = egui::ScrollArea::horizontal()
            .id_salt(shelf_id)
            .auto_shrink([false, true])
            .scroll_offset(Vec2::new(shelf_scroll, 0.0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for card in &cards {
                        let cover = self.artwork_texture(ui.ctx(), card.cover_index);
                        let mut play = false;
                        ui.allocate_ui_with_layout(
                            Vec2::new(182.0, 232.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                let (cover_rect, response) = ui
                                    .allocate_exact_size(Vec2::splat(164.0), egui::Sense::click());
                                let painter = ui.painter();
                                if let Some(cover) = &cover {
                                    painter.image(
                                        cover.id(),
                                        cover_rect,
                                        egui::Rect::from_min_max(
                                            egui::Pos2::ZERO,
                                            egui::pos2(1.0, 1.0),
                                        ),
                                        Color32::WHITE,
                                    );
                                } else {
                                    let palette = [
                                        Color32::from_rgb(111, 44, 59),
                                        Color32::from_rgb(44, 67, 87),
                                        Color32::from_rgb(116, 87, 47),
                                        Color32::from_rgb(67, 48, 91),
                                        Color32::from_rgb(35, 91, 83),
                                    ];
                                    painter.rect_filled(
                                        cover_rect,
                                        egui::CornerRadius::same(10),
                                        palette[card.cover_index % palette.len()],
                                    );
                                    painter.text(
                                        cover_rect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        "♫",
                                        egui::FontId::proportional(56.0),
                                        Color32::from_rgb(235, 230, 235),
                                    );
                                }
                                if response.hovered() {
                                    painter.rect_filled(
                                        cover_rect,
                                        egui::CornerRadius::same(10),
                                        Color32::from_black_alpha(58),
                                    );
                                }
                                let center = cover_rect.center();
                                painter.circle_filled(center, 25.0, Color32::from_black_alpha(150));
                                painter.add(egui::Shape::convex_polygon(
                                    vec![
                                        egui::pos2(center.x - 6.0, center.y - 11.0),
                                        egui::pos2(center.x - 6.0, center.y + 11.0),
                                        egui::pos2(center.x + 12.0, center.y),
                                    ],
                                    Color32::WHITE,
                                    egui::Stroke::NONE,
                                ));
                                if response.clicked() {
                                    play = true;
                                }
                                self.show_home_card_context_menu(&response, card);
                                response.on_hover_text(format!("Play {}", card.title));
                                ui.add_space(8.0);
                                ui.add_sized(
                                    [174.0, 22.0],
                                    egui::Label::new(
                                        RichText::new(&card.title).strong().size(15.0),
                                    )
                                    .truncate(),
                                );
                                ui.add_sized(
                                    [174.0, 34.0],
                                    egui::Label::new(
                                        RichText::new(&card.subtitle)
                                            .size(12.0)
                                            .color(Color32::from_rgb(177, 174, 181)),
                                    )
                                    .truncate(),
                                );
                            },
                        );
                        if play {
                            self.play_indices(card.tracks.clone(), card.start);
                        }
                    }
                });
            });
        self.home_shelf_offsets
            .insert(title.to_string(), output.state.offset.x);
    }

    fn show_track_collection(&mut self, ui: &mut egui::Ui, indices: &[usize], salt: &str) {
        let id = egui::Id::new(("track-collection", salt, self.page as u8));
        const BATCH_SIZE: usize = 50;
        let loaded = self.collection_limit(indices.len());
        if self.view_mode != ViewMode::Grid {
            let row_height = if self.view_mode == ViewMode::Details {
                96.0
            } else {
                84.0
            };
            let output = egui::ScrollArea::vertical()
                .id_salt(id)
                .auto_shrink([false, false])
                .show_rows(ui, row_height, loaded, |ui, row_range| {
                    for row in row_range {
                        self.track_row(ui, indices[row]);
                    }
                });
            self.load_next_collection_batch(
                output.state.offset.y,
                output.inner_rect.height(),
                indices.len(),
                loaded,
                BATCH_SIZE,
                loaded as f32 * row_height,
                row_height * 2.0,
            );
            return;
        }
        let columns = ((ui.available_width() / 190.0).floor() as usize).clamp(1, 8);
        let row_count = loaded.div_ceil(columns);
        let output = egui::ScrollArea::vertical()
            .id_salt(id)
            .auto_shrink([false, false])
            .show_rows(ui, 254.0, row_count, |ui, row_range| {
                for row in row_range {
                    let start = row * columns;
                    let end = (start + columns).min(loaded);
                    ui.horizontal(|ui| {
                        for (offset, &index) in indices[start..end].iter().enumerate() {
                            let cover = self.artwork_texture(ui.ctx(), index);
                            let Some(track) = self.tracks.get(index) else {
                                continue;
                            };
                            let title = track.title.clone();
                            let artist = display_credit(&track.artist);
                            let mut play = false;
                            ui.allocate_ui_with_layout(
                                Vec2::new(176.0, 238.0),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    ui.group(|ui| {
                                        let cover_response = if let Some(cover) = &cover {
                                            ui.add(egui::ImageButton::new((
                                                cover.id(),
                                                Vec2::splat(140.0),
                                            )))
                                        } else {
                                            ui.add_sized([140.0, 140.0], egui::Button::new("♫"))
                                        };
                                        if cover_response.clicked() {
                                            play = true;
                                        }
                                        self.show_track_context_menu(&cover_response, index);
                                        ui.label(RichText::new(&title).strong());
                                        ui.label(
                                            RichText::new(artist).small().color(Color32::GRAY),
                                        );
                                        if ui.button("♥ Like").clicked() {
                                            self.toggle_song_reaction(index, true);
                                        }
                                    });
                                },
                            );
                            if play {
                                self.play_indices(indices.to_vec(), start + offset);
                            }
                        }
                    });
                }
            });
        self.load_next_collection_batch(
            output.state.offset.y,
            output.inner_rect.height(),
            indices.len(),
            loaded,
            BATCH_SIZE,
            row_count as f32 * 254.0,
            508.0,
        );
    }

    fn show_quick_picks(&mut self, ui: &mut egui::Ui, indices: &[usize]) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("A fast look through your collection")
                    .color(Color32::from_rgb(177, 174, 181)),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_sized(
                        [100.0, 40.0],
                        egui::Button::new("▶  Play all").fill(accent_color(&self.accent)),
                    )
                    .clicked()
                {
                    self.play_indices(indices.to_vec(), 0);
                }
            });
        });
        ui.add_space(12.0);
        let columns = 3;
        const BATCH_SIZE: usize = 50;
        let loaded = self.collection_limit(indices.len());
        let row_count = loaded.div_ceil(columns);
        let mut play = None;
        let mut reaction = None;
        let mut delete = None;
        let output = egui::ScrollArea::vertical()
            .id_salt("quick-picks-three-column-list")
            .auto_shrink([false, false])
            .show_rows(ui, 116.0, row_count, |ui, row_range| {
                for row in row_range {
                    let width = (ui.available_width() / columns as f32 - 8.0).max(120.0);
                    ui.horizontal(|ui| {
                        for column in 0..columns {
                            let position = row * columns + column;
                            if position >= loaded {
                                break;
                            }
                            let Some(&index) = indices.get(position) else {
                                break;
                            };
                            let cover = self.artwork_texture(ui.ctx(), index);
                            let Some(track) = self.tracks.get(index) else {
                                continue;
                            };
                            let title = track.title.clone();
                            let artist = display_credit(&track.artist);
                            let album = display_metadata(&track.album);
                            let path = track.path.clone();
                            let liked = self.liked_songs.contains(&path);
                            let disliked = self.disliked_songs.contains(&path);
                            ui.allocate_ui_with_layout(
                                Vec2::new(width, 108.0),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    let art_rect = ui.allocate_space(Vec2::splat(52.0)).1;
                                    if let Some(cover) = &cover {
                                        ui.painter().image(
                                            cover.id(),
                                            art_rect,
                                            egui::Rect::from_min_max(
                                                egui::Pos2::ZERO,
                                                egui::pos2(1.0, 1.0),
                                            ),
                                            Color32::WHITE,
                                        );
                                    } else {
                                        ui.painter().rect_filled(
                                            art_rect,
                                            egui::CornerRadius::same(6),
                                            Color32::from_rgb(48, 36, 44),
                                        );
                                        ui.painter().text(
                                            art_rect.center(),
                                            egui::Align2::CENTER_CENTER,
                                            "♫",
                                            egui::FontId::proportional(24.0),
                                            Color32::WHITE,
                                        );
                                    }
                                    let art_response = ui.interact(
                                        art_rect,
                                        ui.id().with(position),
                                        egui::Sense::click(),
                                    );
                                    if art_response.clicked() {
                                        play = Some(position);
                                    }
                                    self.show_track_context_menu(&art_response, index);
                                    ui.vertical(|ui| {
                                        let title_response = ui.add_sized(
                                            [width - 72.0, 24.0],
                                            egui::Button::new(
                                                RichText::new(&title).strong().size(14.0),
                                            )
                                            .frame(false),
                                        );
                                        if title_response.clicked() {
                                            play = Some(position);
                                        }
                                        title_response.on_hover_text(&artist);
                                        ui.add_sized(
                                            [width - 72.0, 20.0],
                                            egui::Label::new(
                                                RichText::new(format!("{artist} · {album}"))
                                                    .small()
                                                    .color(Color32::from_rgb(177, 174, 181)),
                                            )
                                            .truncate(),
                                        );
                                        ui.horizontal(|ui| {
                                            if ui
                                                .small_button(if liked {
                                                    "♥ Liked"
                                                } else {
                                                    "♡ Like"
                                                })
                                                .clicked()
                                            {
                                                reaction = Some((index, true));
                                            }
                                            if ui
                                                .small_button(if disliked {
                                                    "− Disliked"
                                                } else {
                                                    "− Dislike"
                                                })
                                                .clicked()
                                            {
                                                reaction = Some((index, false));
                                            }
                                            if ui
                                                .small_button("🗑")
                                                .on_hover_text("Delete audio file")
                                                .clicked()
                                            {
                                                delete = Some((path.clone(), title.clone()));
                                            }
                                        });
                                    });
                                },
                            );
                        }
                    });
                    ui.add_space(4.0);
                }
            });
        if let Some(position) = play {
            self.play_indices(indices.to_vec(), position);
        }
        if let Some((index, like)) = reaction {
            self.toggle_song_reaction(index, like);
        }
        if let Some(target) = delete {
            self.pending_delete = Some(target);
        }
        self.load_next_collection_batch(
            output.state.offset.y,
            output.inner_rect.height(),
            indices.len(),
            loaded,
            BATCH_SIZE,
            row_count as f32 * 116.0,
            232.0,
        );
    }

    fn show_playlist_library(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Create and manage your own collections.").color(Color32::GRAY));
        ui.horizontal(|ui| {
            ui.add_sized(
                [300.0, 34.0],
                egui::TextEdit::singleline(&mut self.playlist_name).hint_text("New playlist name"),
            );
            if ui.button("＋ Create playlist").clicked() {
                let name = self.playlist_name.trim().to_string();
                if !name.is_empty() {
                    self.playlists.push(Playlist {
                        name,
                        tracks: Vec::new(),
                    });
                    self.selected_playlist = Some(self.playlists.len() - 1);
                    self.playlist_name.clear();
                    self.save_state();
                }
            }
        });
        ui.add_space(12.0);
        let mut open = None;
        let mut play = false;
        let mut shuffle_play = false;
        let mut enqueue = false;
        let mut delete = None;
        let mut rename = None;
        let mut duplicate = None;
        for (index, playlist) in self.playlists.iter().enumerate() {
            let response = ui.group(|ui| {
                ui.horizontal(|ui| {
                    if ui
                        .selectable_label(
                            false,
                            format!("♫  {} · {} tracks", playlist.name, playlist.tracks.len()),
                        )
                        .clicked()
                    {
                        open = Some(index);
                    }
                    if ui.small_button("▶ Play").clicked() {
                        open = Some(index);
                        play = true;
                    }
                    if ui.small_button("Delete").clicked() {
                        delete = Some(index);
                    }
                })
            });
            response.response.context_menu(|ui| {
                if ui.button("▶  Play playlist").clicked() {
                    open = Some(index);
                    play = true;
                    ui.close_menu();
                }
                if ui.button("🔀  Shuffle and play").clicked() {
                    open = Some(index);
                    shuffle_play = true;
                    ui.close_menu();
                }
                if ui.button("♫  Add playlist to queue").clicked() {
                    open = Some(index);
                    enqueue = true;
                    ui.close_menu();
                }
                if ui.button("Open playlist").clicked() {
                    open = Some(index);
                    ui.close_menu();
                }
                if ui.button("Rename playlist").clicked() {
                    rename = Some(index);
                    ui.close_menu();
                }
                if ui.button("Duplicate playlist").clicked() {
                    duplicate = Some(index);
                    ui.close_menu();
                }
            });
        }
        if let Some(index) = rename {
            if let Some(playlist) = self.playlists.get(index) {
                self.playlist_rename_draft.clone_from(&playlist.name);
                self.renaming_playlist = Some(index);
            }
        }
        let mut save_rename = false;
        let mut cancel_rename = false;
        if let Some(index) = self.renaming_playlist {
            if let Some(playlist) = self.playlists.get(index) {
                ui.horizontal(|ui| {
                    ui.label(format!("Rename “{}”:", playlist.name));
                    ui.add_sized(
                        [260.0, 34.0],
                        egui::TextEdit::singleline(&mut self.playlist_rename_draft),
                    );
                    save_rename = ui.button("Save name").clicked();
                    cancel_rename = ui.button("Cancel").clicked();
                });
            } else {
                self.renaming_playlist = None;
            }
        }
        if save_rename {
            let name = self.playlist_rename_draft.trim().to_string();
            if !name.is_empty() {
                if let Some(playlist) = self
                    .renaming_playlist
                    .and_then(|index| self.playlists.get_mut(index))
                {
                    playlist.name = name;
                    self.save_state();
                    self.status = "Playlist renamed.".into();
                }
            }
            self.renaming_playlist = None;
        } else if cancel_rename {
            self.renaming_playlist = None;
        }
        if let Some(index) = duplicate {
            if let Some(playlist) = self.playlists.get(index).cloned() {
                let mut name = format!("{} Copy", playlist.name);
                let mut suffix = 2;
                while self
                    .playlists
                    .iter()
                    .any(|candidate| candidate.name.eq_ignore_ascii_case(&name))
                {
                    name = format!("{} Copy {suffix}", playlist.name);
                    suffix += 1;
                }
                self.playlists.push(Playlist { name, ..playlist });
                self.save_state();
                self.status = "Playlist duplicated.".into();
            }
        }
        if let Some(index) = open {
            self.selected_playlist = Some(index);
            let tracks = self.indices_for_paths(&self.playlists[index].tracks.clone());
            if play || shuffle_play {
                if !tracks.is_empty() {
                    if shuffle_play {
                        self.play_indices_with_shuffle(tracks, 0, true);
                    } else {
                        self.play_indices(tracks, 0);
                    }
                }
            } else if enqueue {
                self.add_tracks_to_queue(&tracks);
            }
        }
        if let Some(index) = delete {
            self.playlists.remove(index);
            self.selected_playlist = None;
            self.save_state();
        }
    }

    fn show_settings(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
            .id_salt("settings-content")
            .auto_shrink([false, false])
            .show(ui, |ui| self.show_settings_content(ui));
    }

    fn show_settings_content(&mut self, ui: &mut egui::Ui) {
        ui.label(
            RichText::new(
                "Display, playback, and recommendation changes apply on Save. Folder changes take effect immediately.",
            )
            .color(Color32::GRAY),
        );
        ui.add_space(12.0);
        let mut draft = self
            .settings_draft
            .clone()
            .unwrap_or_else(|| self.preferences_snapshot());
        let mut save = false;
        let mut discard = false;
        egui::Grid::new("settings-grid")
            .num_columns(2)
            .spacing([24.0, 18.0])
            .show(ui, |ui| {
                ui.label(RichText::new("UI scale").strong());
                ui.horizontal(|ui| {
                    ui.add(
                        egui::Slider::new(&mut draft.ui_scale, 75.0..=150.0)
                            .step_by(5.0)
                            .suffix("%"),
                    )
                    .changed();
                    if ui.button("Reset").clicked() {
                        draft.ui_scale = default_ui_scale();
                    }
                });
                ui.end_row();
                ui.label(RichText::new("Accent color").strong());
                egui::ComboBox::from_id_salt("accent-color")
                    .selected_text(&draft.accent)
                    .show_ui(ui, |ui| {
                        for (name, _) in ACCENTS {
                            ui.selectable_value(&mut draft.accent, name.to_string(), name);
                        }
                    });
                ui.end_row();
                ui.label(RichText::new("Default view").strong());
                egui::ComboBox::from_id_salt("default-view")
                    .selected_text(view_mode_name(draft.view_mode))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut draft.view_mode, ViewMode::Grid, "Grid");
                        ui.selectable_value(&mut draft.view_mode, ViewMode::Details, "Details");
                        ui.selectable_value(&mut draft.view_mode, ViewMode::List, "List");
                    });
                ui.end_row();
                ui.label(RichText::new("Albums view").strong());
                view_mode_combo(ui, "settings-album-view", &mut draft.album_view_mode);
                ui.end_row();
                ui.label(RichText::new("Artists view").strong());
                view_mode_combo(ui, "settings-artist-view", &mut draft.artist_view_mode);
                ui.end_row();
                ui.label(RichText::new("Recommendations").strong());
                ui.horizontal(|ui| {
                    ui.label("Familiar");
                    ui.add(egui::Slider::new(&mut draft.discovery, 0.0..=1.0).show_value(false));
                    ui.label("Discover");
                });
                ui.end_row();
                ui.label(RichText::new("Unplayed songs").strong());
                ui.checkbox(
                    &mut draft.prefer_unplayed,
                    "Prioritize songs I haven't played",
                );
                ui.end_row();
                ui.label(RichText::new("Mix variety").strong());
                ui.horizontal(|ui| {
                    ui.label("Similar");
                    ui.add(
                        egui::Slider::new(&mut draft.recommendation_variety, 0.0..=1.0)
                            .show_value(false),
                    );
                    ui.label("Varied");
                });
                ui.end_row();
                ui.label(RichText::new("Mix length").strong());
                ui.add(
                    egui::Slider::new(&mut draft.recommendation_size, 20..=100)
                        .step_by(10.0)
                        .suffix(" songs"),
                );
                ui.end_row();
                ui.label(RichText::new("Playback").strong());
                ui.horizontal(|ui| {
                    ui.checkbox(&mut draft.shuffle, "Shuffle");
                    egui::ComboBox::from_id_salt("repeat-mode")
                        .selected_text(repeat_mode_name(draft.repeat))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut draft.repeat, RepeatMode::Off, "Repeat off");
                            ui.selectable_value(&mut draft.repeat, RepeatMode::All, "Repeat all");
                            ui.selectable_value(&mut draft.repeat, RepeatMode::One, "Repeat one");
                        });
                });
                ui.end_row();
                ui.label(RichText::new("Default volume").strong());
                ui.add(
                    egui::Slider::new(&mut draft.volume, 0.0..=1.0)
                        .step_by(0.05)
                        .show_value(true)
                        .custom_formatter(|value, _| format!("{:.0}%", value * 100.0)),
                );
                ui.end_row();
                ui.label(RichText::new("Music folders").strong());
                ui.vertical(|ui| {
                    let mut remove_folder = None;
                    for (index, folder) in self.folders.iter().enumerate() {
                        ui.horizontal(|ui| {
                            ui.label(folder.display().to_string());
                            if ui.small_button("Remove").clicked() {
                                remove_folder = Some(index);
                            }
                        });
                    }
                    ui.horizontal(|ui| {
                        if ui.button("＋ Add folder").clicked() {
                            self.add_folder();
                        }
                        if ui.button("↻ Rescan").clicked() {
                            self.start_scan();
                        }
                    });
                    if let Some(index) = remove_folder {
                        self.folders.remove(index);
                        self.save_state();
                        self.start_scan();
                    }
                });
                ui.end_row();
                ui.label(RichText::new("Library removals").strong());
                ui.horizontal(|ui| {
                    ui.label(format!("{} excluded files", self.excluded_tracks.len()));
                    if ui.button("Restore removed tracks").clicked() {
                        self.excluded_tracks.clear();
                        self.recommendations_dirty = true;
                        self.save_state();
                        self.start_scan();
                    }
                });
                ui.end_row();
            });
        self.settings_draft = Some(draft.clone());
        ui.horizontal(|ui| {
            save = ui
                .add_sized([144.0, 42.0], egui::Button::new("Save changes"))
                .clicked();
            discard = ui
                .add_sized([120.0, 42.0], egui::Button::new("Discard"))
                .clicked();
        });
        if save {
            self.view_mode = draft.view_mode;
            self.album_view_mode = draft.album_view_mode;
            self.artist_view_mode = draft.artist_view_mode;
            self.accent = draft.accent;
            self.shuffle = draft.shuffle;
            self.repeat = draft.repeat;
            self.discovery = draft.discovery;
            self.prefer_unplayed = draft.prefer_unplayed;
            self.recommendation_variety = draft.recommendation_variety.clamp(0.0, 1.0);
            self.recommendation_size = draft.recommendation_size.clamp(20, 100);
            self.ui_scale = draft.ui_scale.clamp(75.0, 150.0);
            self.volume = draft.volume.clamp(0.0, 1.0);
            if let Some(sink) = &self.sink {
                sink.set_volume(self.volume);
            }
            self.recommendations_dirty = true;
            self.settings_draft = None;
            self.save_state();
            self.status = "Settings applied and saved.".into();
        } else if discard {
            self.settings_draft = None;
            self.status = "Unapplied settings changes discarded.".into();
        }
    }

    fn recommendation_indices(&mut self) -> Vec<usize> {
        if self.recommendations_dirty {
            self.recommendations = self.compute_recommendations();
            self.recommendations_dirty = false;
        }
        self.recommendations.clone()
    }

    fn compute_recommendations(&self) -> Vec<usize> {
        let track_by_path = self
            .tracks
            .iter()
            .enumerate()
            .map(|(index, track)| (track.path.as_path(), index))
            .collect::<HashMap<_, _>>();
        let mut artist_affinity = HashMap::<String, f32>::new();
        let mut genre_affinity = HashMap::<String, f32>::new();
        let mut album_affinity = HashMap::<String, f32>::new();
        let mut recent_rank = HashMap::<PathBuf, usize>::new();

        for (rank, path) in self.history.iter().enumerate() {
            recent_rank.insert(path.clone(), rank);
            if rank >= 200 {
                continue;
            }
            let Some(&index) = track_by_path.get(path.as_path()) else {
                continue;
            };
            let track = &self.tracks[index];
            let weight = 2.5 / (1.0 + rank as f32 * 0.035);
            for artist in artist_names(&track.artist) {
                *artist_affinity.entry(artist.to_lowercase()).or_default() += weight;
            }
            if !is_unknown(&track.genre) {
                *genre_affinity
                    .entry(track.genre.trim().to_lowercase())
                    .or_default() += weight * 0.6;
            }
            if !is_unknown(&track.album) {
                *album_affinity
                    .entry(track.album.trim().to_lowercase())
                    .or_default() += weight * 0.35;
            }
        }

        for track in &self.tracks {
            if !self.liked_songs.contains(&track.path) {
                continue;
            }
            for artist in artist_names(&track.artist) {
                *artist_affinity.entry(artist.to_lowercase()).or_default() += 3.0;
            }
            if !is_unknown(&track.genre) {
                *genre_affinity
                    .entry(track.genre.trim().to_lowercase())
                    .or_default() += 1.5;
            }
            if !is_unknown(&track.album) {
                *album_affinity
                    .entry(track.album.trim().to_lowercase())
                    .or_default() += 1.0;
            }
        }

        let mut genre_frequency = HashMap::<String, usize>::new();
        for track in &self.tracks {
            if !is_unknown(&track.genre) {
                *genre_frequency
                    .entry(track.genre.trim().to_lowercase())
                    .or_default() += 1;
            }
        }

        let discovery = self.discovery.clamp(0.0, 1.0);
        let mut remaining = self
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, track)| {
                !self.excluded_tracks.contains(&track.path)
                    && !self.disliked_songs.contains(&track.path)
            })
            .map(|(index, track)| {
                let artist_score = artist_names(&track.artist)
                    .iter()
                    .map(|artist| {
                        let key = artist.to_lowercase();
                        artist_affinity.get(&key).copied().unwrap_or_default()
                            + if self.liked_artists.contains(&key) {
                                4.0
                            } else {
                                0.0
                            }
                    })
                    .fold(0.0_f32, f32::max)
                    * (1.0 - discovery * 0.55);
                let genre_key = track.genre.trim().to_lowercase();
                let genre_score =
                    genre_affinity.get(&genre_key).copied().unwrap_or_default() * (1.0 - discovery);
                let genre_rarity = genre_frequency
                    .get(&genre_key)
                    .copied()
                    .map_or(0.0, |count| 1.0 / (count as f32).sqrt())
                    * discovery
                    * 2.0;
                let album_key = track.album.trim().to_lowercase();
                let album_score = album_affinity.get(&album_key).copied().unwrap_or_default()
                    + if self.liked_albums.contains(&album_key) {
                        2.0
                    } else {
                        0.0
                    };
                let played = recent_rank.get(&track.path).copied();
                let unplayed_bonus = if played.is_none() && self.prefer_unplayed {
                    3.5
                } else {
                    0.0
                };
                let recent_penalty = played.map_or(0.0, |rank| match rank {
                    0..=4 => 4.0,
                    5..=14 => 2.0,
                    15..=39 => 0.8,
                    _ => 0.0,
                });
                let liked_bonus = if self.liked_songs.contains(&track.path) {
                    1.5
                } else {
                    0.0
                };
                (
                    index,
                    artist_score
                        + genre_score
                        + genre_rarity
                        + album_score
                        + liked_bonus
                        + unplayed_bonus
                        - recent_penalty,
                )
            })
            .collect::<Vec<_>>();

        let mut recommendations =
            Vec::<usize>::with_capacity(self.recommendation_size.clamp(20, 100));
        while !remaining.is_empty()
            && recommendations.len() < self.recommendation_size.clamp(20, 100)
        {
            let (best_position, _) = remaining
                .iter()
                .enumerate()
                .map(|(position, &(index, score))| {
                    let track = &self.tracks[index];
                    let variety_penalty = recommendations.iter().fold(0.0, |penalty, &picked| {
                        let picked_track = &self.tracks[picked];
                        let same_artist = !is_unknown(&track.artist)
                            && track.artist.eq_ignore_ascii_case(&picked_track.artist);
                        let same_album = !is_unknown(&track.album)
                            && track.album.eq_ignore_ascii_case(&picked_track.album);
                        penalty
                            + if same_artist {
                                1.8 * self.recommendation_variety
                            } else {
                                0.0
                            }
                            + if same_album {
                                2.5 * self.recommendation_variety
                            } else {
                                0.0
                            }
                    });
                    (position, score - variety_penalty)
                })
                .max_by(|left, right| {
                    left.1
                        .total_cmp(&right.1)
                        .then_with(|| right.0.cmp(&left.0))
                })
                .expect("remaining candidates are non-empty");
            let (index, _) = remaining.swap_remove(best_position);
            recommendations.push(index);
        }
        recommendations
    }

    fn artwork_texture(
        &mut self,
        context: &egui::Context,
        track_index: usize,
    ) -> Option<TextureHandle> {
        let track = self.tracks.get(track_index)?;
        let path = track.path.clone();
        if let Some(texture) = self.textures.get(&path).cloned() {
            self.texture_usage
                .retain(|cached_path| cached_path != &path);
            self.texture_usage.push_back(path);
            return Some(texture);
        }
        if self.artwork_pending.len() < 4
            && !self.artwork_pending.contains(&path)
            && !self.artwork_failed.contains(&path)
        {
            let Some(bytes) = self
                .tracks
                .get(track_index)
                .and_then(|track| track.artwork.clone())
            else {
                return None;
            };
            let sender = self.artwork_sender.clone();
            let request_path = path.clone();
            self.artwork_pending.insert(path);
            thread::spawn(move || {
                let result = image::load_from_memory(&bytes)
                    .map(|image| image.thumbnail(192, 192).to_rgba8());
                let (size, rgba) = match result {
                    Ok(decoded) => (
                        [decoded.width() as usize, decoded.height() as usize],
                        Some(decoded.into_raw()),
                    ),
                    Err(_) => ([0, 0], None),
                };
                let _ = sender.send(ArtworkResult {
                    path: request_path,
                    size,
                    rgba,
                });
            });
            context.request_repaint_after(Duration::from_millis(50));
        }
        None
    }
}

impl eframe::App for PlayerApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        #[cfg(target_os = "windows")]
        self.initialize_media_controls(frame, ctx);
        self.poll_scan();
        self.poll_artwork(ctx);
        self.poll_media_key_commands();
        if !self.artwork_pending.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        if self.sink.as_ref().is_some_and(Sink::empty) {
            self.next_track();
        }
        #[cfg(target_os = "windows")]
        self.update_media_session();
        let can_use_shortcuts = !ctx.wants_keyboard_input();
        let shortcuts = ctx.input(|input| {
            (
                can_use_shortcuts
                    && (input.key_pressed(egui::Key::Space)
                        || (input.modifiers.ctrl && input.key_pressed(egui::Key::P))),
                can_use_shortcuts
                    && (input.key_pressed(egui::Key::ArrowRight)
                        || (input.modifiers.ctrl && input.key_pressed(egui::Key::ArrowRight))),
                can_use_shortcuts
                    && (input.key_pressed(egui::Key::ArrowLeft)
                        || (input.modifiers.ctrl && input.key_pressed(egui::Key::ArrowLeft))),
                (input.modifiers.ctrl && input.key_pressed(egui::Key::F))
                    || (can_use_shortcuts
                        && !input.modifiers.any()
                        && input.key_pressed(egui::Key::Slash)),
            )
        });
        if shortcuts.0 {
            self.toggle_playback();
        }
        if shortcuts.1 {
            self.next_track();
        }
        if shortcuts.2 {
            self.previous_track();
        }
        if shortcuts.3 {
            ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("global-search")));
        }
        ctx.set_zoom_factor(self.ui_scale / 100.0);
        if self.scan_busy
            || self
                .sink
                .as_ref()
                .is_some_and(|sink| !sink.is_paused() && !sink.empty())
        {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        ctx.set_visuals(egui::Visuals::dark());
        let accent = accent_color(&self.accent);
        ctx.style_mut(|style| {
            style.visuals.panel_fill = Color32::from_rgb(0, 0, 0);
            style.visuals.window_fill = Color32::from_rgb(19, 19, 19);
            style.visuals.extreme_bg_color = Color32::from_rgb(19, 19, 19);
            style.visuals.faint_bg_color = Color32::from_rgb(24, 24, 24);
            style.visuals.selection.bg_fill = accent;
            style.visuals.widgets.active.bg_fill = accent;
            style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(36, 44, 66);
            style.spacing.item_spacing = Vec2::new(12.0, 10.0);
            style.spacing.interact_size = Vec2::new(42.0, 40.0);
            style.interaction.selectable_labels = false;
            style
                .text_styles
                .insert(egui::TextStyle::Body, egui::FontId::proportional(15.0));
            style
                .text_styles
                .insert(egui::TextStyle::Button, egui::FontId::proportional(14.0));
            style
                .text_styles
                .insert(egui::TextStyle::Small, egui::FontId::proportional(12.0));
            style
                .text_styles
                .insert(egui::TextStyle::Heading, egui::FontId::proportional(29.0));
        });

        egui::TopBottomPanel::top("top-bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading(RichText::new("♫ LILT").color(accent).strong().size(22.0));
                ui.add_space(14.0);
                ui.add_sized(
                    [420.0, 36.0],
                    egui::TextEdit::singleline(&mut self.query)
                        .id(egui::Id::new("global-search"))
                        .hint_text("Search songs, artists, albums…"),
                );
                if ui.button("＋ Add folder").clicked() {
                    self.add_folder();
                }
                if ui.button("↻ Rescan").clicked() {
                    self.start_scan();
                }
                if ui
                    .selectable_label(
                        !self.queue_collapsed,
                        format!("♫ Queue  {}", self.queue.len()),
                    )
                    .clicked()
                {
                    self.queue_collapsed = !self.queue_collapsed;
                }
                if ui
                    .selectable_label(self.page == Page::Settings, "⚙ Settings")
                    .clicked()
                {
                    self.page = Page::Settings;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(&self.scan_status)
                            .small()
                            .color(Color32::GRAY),
                    );
                });
            });
        });

        egui::SidePanel::left("navigation")
            .resizable(false)
            .default_width(200.0)
            .show(ctx, |ui| {
                ui.add_space(16.0);
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for (label, items) in [
                        (
                            "DISCOVER",
                            vec![
                                (Page::Home, "⌂  Home"),
                                (Page::Songs, "♫  Songs"),
                                (Page::Albums, "▦  Albums"),
                                (Page::Artists, "♙  Artists"),
                            ],
                        ),
                        (
                            "YOUR MUSIC",
                            vec![
                                (Page::LikedSongs, "♥  Liked music"),
                                (Page::LikedArtists, "♙  Liked artists"),
                                (Page::LikedAlbums, "▣  Saved albums"),
                            ],
                        ),
                        (
                            "ACTIVITY",
                            vec![
                                (Page::History, "◷  History"),
                                (Page::Disliked, "−  Disliked"),
                                (Page::Lyrics, "≋  Lyrics"),
                                (Page::Settings, "⚙ Settings"),
                            ],
                        ),
                    ] {
                        ui.label(
                            RichText::new(label)
                                .small()
                                .strong()
                                .color(Color32::from_rgb(150, 145, 151)),
                        );
                        ui.add_space(5.0);
                        for (page, title) in items {
                            if ui
                                .add_sized(
                                    [ui.available_width(), 42.0],
                                    egui::Button::new(title).selected(self.page == page),
                                )
                                .clicked()
                            {
                                self.page = page;
                                self.selected_playlist = None;
                            }
                        }
                        ui.add_space(12.0);
                    }
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("PLAYLISTS")
                                .small()
                                .strong()
                                .color(Color32::from_rgb(150, 145, 151)),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .add_sized([34.0, 32.0], egui::Button::new("＋"))
                                .on_hover_text("Create a playlist")
                                .clicked()
                            {
                                self.page = Page::Playlists;
                                self.selected_playlist = None;
                            }
                        });
                    });
                    let playlist_names = self
                        .playlists
                        .iter()
                        .map(|playlist| playlist.name.clone())
                        .collect::<Vec<_>>();
                    for (index, playlist_name) in playlist_names.iter().enumerate() {
                        let selected =
                            self.page == Page::Playlists && self.selected_playlist == Some(index);
                        if ui
                            .add_sized(
                                [ui.available_width(), 42.0],
                                egui::Button::new(format!("♫  {playlist_name}")).selected(selected),
                            )
                            .clicked()
                        {
                            self.page = Page::Playlists;
                            self.selected_playlist = Some(index);
                        }
                    }
                    ui.add_space(12.0);
                    ui.separator();
                    ui.label(
                        RichText::new(format!(
                            "{} songs · {} folders",
                            self.tracks.len(),
                            self.folders.len()
                        ))
                        .small()
                        .color(Color32::GRAY),
                    );
                });
            });

        if !self.queue_collapsed {
            egui::SidePanel::right("up-next")
                .resizable(true)
                .default_width(350.0)
                .width_range(285.0..=480.0)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        if ui
                            .add_sized([38.0, 38.0], egui::Button::new("×"))
                            .on_hover_text("Collapse Up next")
                            .clicked()
                        {
                            self.queue_collapsed = true;
                        }
                        ui.vertical(|ui| {
                            ui.heading("Up next");
                            ui.label(
                                RichText::new(format!("{} songs in queue", self.queue.len()))
                                    .small()
                                    .color(Color32::GRAY),
                            );
                        });
                        if ui
                            .add_sized([64.0, 38.0], egui::Button::new("Clear"))
                            .on_hover_text("Clear the queue and stop playback")
                            .clicked()
                        {
                            self.queue.clear();
                            self.current = None;
                            self.sink = None;
                            self.stream = None;
                        }
                    });
                    ui.horizontal(|ui| {
                        if ui
                            .selectable_label(self.shuffle, "🔀 Shuffle")
                            .on_hover_text("Toggle queue shuffle")
                            .clicked()
                        {
                            self.shuffle = !self.shuffle;
                            if self.shuffle && self.current.is_some() {
                                let current = self.current.unwrap_or(0);
                                let selected = self.queue.get(current).copied();
                                if let Some(selected) = selected {
                                    self.queue.remove(current);
                                    shuffle_slice(&mut self.queue);
                                    self.queue.insert(0, selected);
                                    self.current = Some(0);
                                }
                            }
                            self.save_state();
                        }
                        if ui
                            .selectable_label(
                                self.repeat != RepeatMode::Off,
                                format!("↻ {}", repeat_mode_name(self.repeat)),
                            )
                            .on_hover_text(format!("Repeat: {}", repeat_mode_name(self.repeat)))
                            .clicked()
                        {
                            self.repeat = match self.repeat {
                                RepeatMode::Off => RepeatMode::All,
                                RepeatMode::All => RepeatMode::One,
                                RepeatMode::One => RepeatMode::Off,
                            };
                            self.save_state();
                        }
                    });
                    ui.separator();
                    let current_position = self.current;
                    let current_track_index =
                        current_position.and_then(|position| self.queue.get(position).copied());
                    if let Some(track_index) = current_track_index {
                        let cover = self.artwork_texture(ui.ctx(), track_index);
                        let details = self.tracks.get(track_index).map(|track| {
                            (
                                track.title.clone(),
                                display_credit(&track.artist),
                                display_metadata(&track.album),
                            )
                        });
                        if let Some((title, artist, album)) = details {
                            egui::Frame::new()
                                .fill(Color32::from_rgb(24, 28, 40))
                                .stroke(egui::Stroke::new(
                                    1.0_f32,
                                    accent_color(&self.accent).linear_multiply(0.35),
                                ))
                                .inner_margin(10)
                                .corner_radius(egui::CornerRadius::same(12))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        if let Some(cover) = &cover {
                                            ui.image((cover.id(), Vec2::splat(56.0)));
                                        }
                                        ui.vertical(|ui| {
                                            ui.label(
                                                RichText::new("NOW PLAYING")
                                                    .small()
                                                    .strong()
                                                    .color(accent_color(&self.accent)),
                                            );
                                            ui.add_sized(
                                                [ui.available_width(), 22.0],
                                                egui::Label::new(RichText::new(title).strong())
                                                    .truncate(),
                                            );
                                            ui.add_sized(
                                                [ui.available_width(), 20.0],
                                                egui::Label::new(
                                                    RichText::new(format!("{artist} · {album}"))
                                                        .small()
                                                        .color(Color32::GRAY),
                                                )
                                                .truncate(),
                                            );
                                        });
                                    });
                                });
                        }
                    }
                    let upcoming_start = current_position.map_or(0, |position| position + 1);
                    let upcoming_count = self.queue.len().saturating_sub(upcoming_start);
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("COMING UP").small().strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{upcoming_count} tracks"))
                                    .small()
                                    .color(Color32::GRAY),
                            );
                        });
                    });
                    let mut play_queue_position = None;
                    let mut remove_queue_position = None;
                    let mut move_queue = None;
                    egui::ScrollArea::vertical()
                        .id_salt("up-next-tracks")
                        .auto_shrink([false, false])
                        .show_rows(ui, 72.0, upcoming_count, |ui, row_range| {
                            for row in row_range {
                                let position = upcoming_start + row;
                                let track_index = self.queue[position];
                                let Some(track) = self.tracks.get(track_index) else {
                                    continue;
                                };
                                let track_title = track.title.clone();
                                let track_artist = display_credit(&track.artist);
                                let cover = self.artwork_texture(ui.ctx(), track_index);
                                ui.add_space(5.0);
                                egui::Frame::new()
                                    .fill(Color32::from_rgb(19, 19, 23))
                                    .stroke(egui::Stroke::new(
                                        1.0_f32,
                                        Color32::from_rgb(37, 38, 45),
                                    ))
                                    .inner_margin(8)
                                    .corner_radius(egui::CornerRadius::same(10))
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            if let Some(cover) = &cover {
                                                ui.image((cover.id(), Vec2::splat(46.0)));
                                            } else {
                                                ui.add_sized(
                                                    [46.0, 46.0],
                                                    egui::Button::new(
                                                        RichText::new("♫")
                                                            .color(accent_color(&self.accent)),
                                                    ),
                                                );
                                            }
                                            let text_width =
                                                (ui.available_width() - 184.0).max(50.0);
                                            ui.vertical(|ui| {
                                                let title_response = ui.add_sized(
                                                    [text_width, 24.0],
                                                    egui::Button::new(
                                                        RichText::new(&track_title).strong(),
                                                    )
                                                    .frame(false),
                                                );
                                                if title_response.clicked() {
                                                    play_queue_position = Some(position);
                                                }
                                                title_response.on_hover_text(&track_title);
                                                ui.add_sized(
                                                    [text_width, 20.0],
                                                    egui::Label::new(
                                                        RichText::new(track_artist)
                                                            .small()
                                                            .color(Color32::GRAY),
                                                    )
                                                    .truncate(),
                                                );
                                            });
                                            if ui
                                                .add_sized([36.0, 38.0], egui::Button::new("▶"))
                                                .on_hover_text("Play this song")
                                                .clicked()
                                            {
                                                play_queue_position = Some(position);
                                            }
                                            if ui
                                                .add_sized([32.0, 36.0], egui::Button::new("↑"))
                                                .on_hover_text("Move up")
                                                .clicked()
                                                && position > upcoming_start
                                            {
                                                move_queue = Some((position, position - 1));
                                            }
                                            if ui
                                                .add_sized([32.0, 36.0], egui::Button::new("↓"))
                                                .on_hover_text("Move down")
                                                .clicked()
                                                && position + 1 < self.queue.len()
                                            {
                                                move_queue = Some((position, position + 1));
                                            }
                                            if ui
                                                .add_sized([36.0, 38.0], egui::Button::new("×"))
                                                .on_hover_text("Remove from queue")
                                                .clicked()
                                            {
                                                remove_queue_position = Some(position);
                                            }
                                        });
                                    });
                            }
                        });
                    if let Some(position) = play_queue_position {
                        self.play_queue_at(position);
                    }
                    if let Some((first, second)) = move_queue {
                        self.swap_queue_positions(first, second);
                    }
                    if let Some(position) = remove_queue_position {
                        let was_current = self.current == Some(position);
                        self.queue.remove(position);
                        if was_current {
                            self.sink = None;
                            self.stream = None;
                            self.current = None;
                            if !self.queue.is_empty() {
                                self.play_queue_at(position.min(self.queue.len() - 1));
                            }
                        } else if let Some(current) = self.current {
                            if position < current {
                                self.current = Some(current - 1);
                            }
                        }
                    }
                    if upcoming_count == 0 && current_track_index.is_none() {
                        ui.add_space(22.0);
                        ui.vertical_centered(|ui| {
                            ui.label(RichText::new("Your queue is empty").strong().size(17.0));
                            ui.label(
                                RichText::new("Play a mix or add songs from your library.")
                                    .small()
                                    .color(Color32::GRAY),
                            );
                        });
                    }
                });
        }

        egui::TopBottomPanel::bottom("player-controls")
            .exact_height(92.0)
            .frame(egui::Frame::new().fill(Color32::BLACK).inner_margin(0))
            .show(ctx, |ui| {
                ui.add_space(4.0);
                egui::Frame::new()
                    .fill(Color32::from_rgb(16, 17, 20))
                    .stroke(egui::Stroke::new(1.0_f32, Color32::from_rgb(38, 40, 46)))
                    .corner_radius(egui::CornerRadius::same(10))
                    .inner_margin(egui::Margin::symmetric(12, 4))
                    .show(ui, |ui| {
                        let duration = self
                            .current
                            .and_then(|position| self.queue.get(position))
                            .and_then(|&index| self.tracks.get(index))
                            .map_or(0.0, |track| track.duration.max(0.0));
                        let mut seek_seconds = (self.playback_offset.as_secs_f64()
                            + self
                                .sink
                                .as_ref()
                                .map_or(0.0, |sink| sink.get_pos().as_secs_f64()))
                        .min(duration);
                        let current_track_index = self
                            .current
                            .and_then(|position| self.queue.get(position))
                            .copied();
                        let current_track_liked = current_track_index
                            .and_then(|index| self.tracks.get(index))
                            .is_some_and(|track| self.liked_songs.contains(&track.path));
                        let current_text = self
                            .current
                            .and_then(|position| self.queue.get(position))
                            .and_then(|&index| self.tracks.get(index))
                            .map(|track| {
                                format!("{} · {}", track.title, display_credit(&track.artist))
                            })
                            .unwrap_or_else(|| "Nothing playing".into());
                        let mut like_current = false;
                        ui.vertical(|ui| {
                            let can_seek = duration > 0.0 && self.sink.is_some();
                            let (progress_rect, response) = ui.allocate_exact_size(
                                Vec2::new(ui.available_width(), 18.0),
                                if can_seek {
                                    egui::Sense::click_and_drag()
                                } else {
                                    egui::Sense::hover()
                                },
                            );
                            let dragging = response.dragged();
                            if can_seek
                                && (dragging || response.drag_stopped() || response.clicked())
                            {
                                if let Some(pointer) =
                                    response.interact_pointer_pos().or_else(|| {
                                        ui.ctx().input(|input| input.pointer.interact_pos())
                                    })
                                {
                                    if let Some(position) = seek_position_from_pointer(
                                        pointer.x,
                                        progress_rect.left(),
                                        progress_rect.width(),
                                        duration,
                                    ) {
                                        seek_seconds = position;
                                        if response.drag_stopped() || response.clicked() {
                                            self.seek_current_to(Duration::from_secs_f64(position));
                                        }
                                    }
                                }
                            }
                            let progress = if duration > 0.0 {
                                (seek_seconds / duration).clamp(0.0, 1.0) as f32
                            } else {
                                0.0
                            };
                            let progress_y = progress_rect.center().y;
                            let progress_end =
                                progress_rect.left() + progress_rect.width() * progress;
                            let track_color = Color32::from_rgb(95, 103, 107);
                            let played_color = Color32::from_rgb(255, 0, 51);
                            ui.painter().line_segment(
                                [
                                    egui::pos2(progress_rect.left(), progress_y),
                                    egui::pos2(progress_rect.right(), progress_y),
                                ],
                                egui::Stroke::new(2.0_f32, track_color),
                            );
                            if progress > 0.0 {
                                ui.painter().line_segment(
                                    [
                                        egui::pos2(progress_rect.left(), progress_y),
                                        egui::pos2(progress_end, progress_y),
                                    ],
                                    egui::Stroke::new(2.5_f32, played_color),
                                );
                            }
                            if response.hovered() || dragging {
                                ui.painter().circle_filled(
                                    egui::pos2(progress_end, progress_y),
                                    5.0,
                                    played_color,
                                );
                                if can_seek {
                                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                }
                            }
                            response.on_hover_text(format!(
                                "{} / {}",
                                format_time(seek_seconds),
                                format_time(duration)
                            ));
                            ui.horizontal_centered(|ui| {
                                ui.add_sized(
                                    [190.0, 24.0],
                                    egui::Label::new(RichText::new(current_text).strong())
                                        .truncate(),
                                );
                                if ui
                                    .add_sized([36.0, 34.0], egui::Button::new("⏮"))
                                    .on_hover_text("Previous track (Left Arrow)")
                                    .clicked()
                                {
                                    self.previous_track();
                                }
                                let label =
                                    if self.sink.as_ref().is_some_and(|sink| !sink.is_paused()) {
                                        "⏸"
                                    } else {
                                        "▶"
                                    };
                                let playing =
                                    self.sink.as_ref().is_some_and(|sink| !sink.is_paused());
                                let play_animation = ctx.animate_bool_with_time(
                                    egui::Id::new("player-play-animation"),
                                    playing,
                                    0.18,
                                );
                                if ui
                                    .add(
                                        egui::Button::new(label)
                                            .fill(accent)
                                            .min_size(Vec2::new(62.0 + play_animation * 4.0, 34.0)),
                                    )
                                    .on_hover_text("Play or pause (Space or Ctrl+P)")
                                    .clicked()
                                {
                                    self.toggle_playback();
                                }
                                if ui
                                    .add_sized([36.0, 34.0], egui::Button::new("⏭"))
                                    .on_hover_text("Next track (Right Arrow)")
                                    .clicked()
                                {
                                    self.next_track();
                                }
                                let heart = if current_track_liked { "♥" } else { "♡" };
                                if ui
                                    .add_enabled(
                                        current_track_index.is_some(),
                                        egui::Button::new(RichText::new(heart).size(20.0).color(
                                            if current_track_liked {
                                                accent
                                            } else {
                                                Color32::GRAY
                                            },
                                        ))
                                        .frame(false),
                                    )
                                    .on_hover_text(if current_track_liked {
                                        "Remove from liked music"
                                    } else {
                                        "Add to liked music"
                                    })
                                    .clicked()
                                {
                                    like_current = true;
                                }
                                ui.add_sized(
                                    [84.0, 20.0],
                                    egui::Label::new(
                                        RichText::new(format!(
                                            "{} / {}",
                                            format_time(seek_seconds),
                                            format_time(duration)
                                        ))
                                        .small()
                                        .color(Color32::GRAY),
                                    ),
                                );
                                if ui
                                    .selectable_label(self.shuffle, "🔀")
                                    .on_hover_text("Shuffle")
                                    .clicked()
                                {
                                    self.shuffle = !self.shuffle;
                                    self.save_state();
                                }
                                if ui
                                    .selectable_label(
                                        self.repeat != RepeatMode::Off,
                                        repeat_mode_name(self.repeat),
                                    )
                                    .on_hover_text("Repeat")
                                    .clicked()
                                {
                                    self.repeat = match self.repeat {
                                        RepeatMode::Off => RepeatMode::All,
                                        RepeatMode::All => RepeatMode::One,
                                        RepeatMode::One => RepeatMode::Off,
                                    };
                                    self.save_state();
                                }
                                ui.label(
                                    RichText::new("🔊")
                                        .size(14.0)
                                        .color(Color32::from_rgb(175, 179, 190)),
                                );
                                let volume_response = ui
                                    .scope(|ui| {
                                        ui.visuals_mut().selection.bg_fill = accent;
                                        ui.spacing_mut().slider_rail_height = 3.0;
                                        ui.add_sized(
                                            [104.0, 14.0],
                                            egui::Slider::new(&mut self.volume, 0.0..=1.0)
                                                .show_value(false),
                                        )
                                    })
                                    .inner
                                    .on_hover_text(format!("Volume: {:.0}%", self.volume * 100.0));
                                if volume_response.changed() {
                                    if let Some(sink) = &self.sink {
                                        sink.set_volume(self.volume);
                                    }
                                    self.save_state();
                                }
                            });
                        });
                        if like_current {
                            if let Some(index) = current_track_index {
                                self.toggle_song_reaction(index, true);
                            }
                        }
                    });
                ui.add_space(6.0);
            });

        egui::CentralPanel::default().show(ctx, |ui| self.show_library(ui));

        if let Some((path, title)) = self.pending_delete.clone() {
            let mut confirm = false;
            let mut cancel = false;
            egui::Window::new("Delete audio file?")
                .id(egui::Id::new("confirm-audio-delete"))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label(RichText::new(format!("Delete “{title}” permanently?")).strong());
                    ui.label(
                        RichText::new(path.display().to_string())
                            .small()
                            .color(Color32::GRAY),
                    );
                    ui.colored_label(
                        Color32::LIGHT_RED,
                        "This removes the audio file from disk and cannot be undone.",
                    );
                    ui.horizontal(|ui| {
                        confirm = ui
                            .add_sized([144.0, 40.0], egui::Button::new("Delete permanently"))
                            .clicked();
                        cancel = ui
                            .add_sized([100.0, 40.0], egui::Button::new("Cancel"))
                            .clicked();
                    });
                });
            if confirm {
                self.pending_delete = None;
                self.delete_audio_file(&path);
            } else if cancel {
                self.pending_delete = None;
            }
        }
    }
}

fn scan_folders(folders: &[PathBuf]) -> ScanResult {
    let mut tracks = Vec::new();
    let mut errors = Vec::new();
    let mut pending = folders.to_vec();
    while let Some(folder) = pending.pop() {
        let entries = match fs::read_dir(&folder) {
            Ok(entries) => entries,
            Err(error) => {
                errors.push(format!("{}: {error}", folder.display()));
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    errors.push(format!("{}: {error}", folder.display()));
                    continue;
                }
            };
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if is_audio_file(&path) {
                tracks.push(read_track(path));
            }
        }
    }
    tracks.sort_by(|left, right| {
        left.artist
            .to_lowercase()
            .cmp(&right.artist.to_lowercase())
            .then_with(|| left.album.to_lowercase().cmp(&right.album.to_lowercase()))
            .then_with(|| left.title.to_lowercase().cmp(&right.title.to_lowercase()))
    });
    ScanResult {
        tracks,
        error: if errors.is_empty() {
            None
        } else {
            Some(format!(
                "Some folders could not be scanned: {}",
                errors.join("; ")
            ))
        },
    }
}

fn read_track(path: PathBuf) -> Track {
    let fallback_title = path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("Untitled")
        .to_string();
    let mut track = Track {
        path: path.clone(),
        title: fallback_title,
        added: fs::metadata(&path)
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |duration| duration.as_secs()),
        ..Track::default()
    };
    if let Ok(tagged) = Probe::open(&path).and_then(|probe| probe.read()) {
        track.duration = tagged.properties().duration().as_secs_f64();
        if let Some(tag) = tagged.primary_tag() {
            if let Some(value) = tag.title() {
                if !is_unknown(value.as_ref()) {
                    track.title = value.to_string();
                }
            }
            track.year = tag.year().unwrap_or_default();
            track.artist = tag.artist().unwrap_or_default().to_string();
            track.album = tag.album().unwrap_or_default().to_string();
            track.genre = tag.genre().unwrap_or_default().to_string();
            track.artwork = tag
                .pictures()
                .first()
                .map(|picture| picture.data().to_vec());
        }
    }
    track.search_text = format!(
        "{} {} {} {}",
        track.title, track.artist, track.album, track.genre
    )
    .to_lowercase();
    track
}

fn read_lyrics(track_path: &Path) -> Result<Option<String>, String> {
    let lyrics_path = track_path.with_extension("lrc");
    match fs::read_to_string(&lyrics_path) {
        Ok(lyrics) => Ok(Some(lyrics)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "Could not read lyrics at {}: {error}",
            lyrics_path.display()
        )),
    }
}

fn is_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| AUDIO_EXTENSIONS.contains(&extension.to_lowercase().as_str()))
}

fn is_unknown(value: &str) -> bool {
    matches!(
        value.trim().to_lowercase().as_str(),
        "" | "unknown"
            | "unknown artist"
            | "unknown album"
            | "unknown genre"
            | "n/a"
            | "na"
            | "none"
            | "null"
            | "various artists"
            | "various"
            | "?"
            | "-"
    )
}

fn display_metadata(value: &str) -> String {
    if is_unknown(value) {
        "—".into()
    } else {
        value.trim().into()
    }
}

fn artist_names(credit: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut seen = HashSet::new();
    for name in credit.split([',', ';']) {
        let name = name.trim();
        if !is_unknown(name) && seen.insert(name.to_lowercase()) {
            names.push(name.to_string());
        }
    }
    names
}

fn merge_credits<'a>(credits: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut artists = Vec::new();
    for credit in credits {
        for name in artist_names(credit) {
            if seen.insert(name.to_lowercase()) {
                artists.push(name);
            }
        }
    }
    artists
}

fn display_credit(credit: &str) -> String {
    let artists = artist_names(credit);
    if artists.is_empty() {
        "—".into()
    } else {
        artists.join(", ")
    }
}

fn format_time(seconds: f64) -> String {
    let seconds = seconds.max(0.0) as u64;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

fn seek_position_from_pointer(
    pointer_x: f32,
    track_left: f32,
    track_width: f32,
    duration: f64,
) -> Option<f64> {
    if !pointer_x.is_finite()
        || !track_left.is_finite()
        || !track_width.is_finite()
        || track_width <= 0.0
        || !duration.is_finite()
        || duration <= 0.0
    {
        return None;
    }
    let progress = ((pointer_x - track_left) / track_width).clamp(0.0, 1.0) as f64;
    Some(duration * progress)
}

fn preferences_path() -> Result<PathBuf, String> {
    let project = ProjectDirs::from("com", "Lilt", "Lilt Player")
        .ok_or_else(|| "Could not locate the user configuration directory.".to_string())?;
    fs::create_dir_all(project.config_dir())
        .map_err(|error| format!("Could not create settings directory: {error}"))?;
    Ok(project.config_dir().join("settings.json"))
}

fn load_preferences() -> Result<Preferences, String> {
    let path = preferences_path()?;
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Preferences::default());
        }
        Err(error) => {
            return Err(format!(
                "Could not read settings at {}: {error}",
                path.display()
            ));
        }
    };
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("Could not parse settings at {}: {error}", path.display()))
}

fn save_preferences(preferences: &Preferences) -> Result<(), String> {
    let path = preferences_path()?;
    let bytes = serde_json::to_vec_pretty(&preferences)
        .map_err(|error| format!("Could not encode settings: {error}"))?;
    fs::write(&path, bytes)
        .map_err(|error| format!("Could not save settings at {}: {error}", path.display()))
}

fn view_mode_name(mode: ViewMode) -> &'static str {
    match mode {
        ViewMode::Grid => "Grid",
        ViewMode::Details => "Details",
        ViewMode::List => "List",
    }
}

fn view_mode_combo(ui: &mut egui::Ui, id: &str, mode: &mut ViewMode) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt(id)
        .selected_text(view_mode_name(*mode))
        .show_ui(ui, |ui| {
            changed |= ui.selectable_value(mode, ViewMode::Grid, "Grid").changed();
            changed |= ui
                .selectable_value(mode, ViewMode::Details, "Details")
                .changed();
            changed |= ui.selectable_value(mode, ViewMode::List, "List").changed();
        });
    changed
}

fn paint_cover_or_placeholder(
    ui: &egui::Ui,
    cover: Option<&TextureHandle>,
    rect: egui::Rect,
    seed: usize,
) {
    if let Some(cover) = cover {
        ui.painter().image(
            cover.id(),
            rect,
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    } else {
        let palette = [
            Color32::from_rgb(70, 38, 54),
            Color32::from_rgb(38, 54, 72),
            Color32::from_rgb(78, 64, 39),
            Color32::from_rgb(55, 42, 75),
            Color32::from_rgb(34, 69, 61),
        ];
        ui.painter().rect_filled(
            rect,
            egui::CornerRadius::same(8),
            palette[seed % palette.len()],
        );
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "♫",
            egui::FontId::proportional((rect.width() * 0.32).clamp(20.0, 42.0)),
            Color32::WHITE,
        );
    }
}

fn repeat_mode_name(mode: RepeatMode) -> &'static str {
    match mode {
        RepeatMode::Off => "↻ Off",
        RepeatMode::All => "↻ All",
        RepeatMode::One => "↻¹ One",
    }
}

fn accent_color(name: &str) -> Color32 {
    ACCENTS
        .iter()
        .find(|(candidate, _)| *candidate == name)
        .map_or(ACCENTS[0].1, |(_, color)| *color)
}

fn shuffle_slice<T>(items: &mut [T]) {
    let mut state = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    if state == 0 {
        state = 0x9E37_79B9_7F4A_7C15;
    }
    for index in (1..items.len()).rev() {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        items.swap(index, (state as usize) % (index + 1));
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(APP_NAME)
            .with_inner_size([1680.0, 1040.0])
            .with_min_inner_size([1120.0, 720.0]),
        ..Default::default()
    };
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(|_creation_context| Ok(Box::new(PlayerApp::new()))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn unknown_metadata_is_not_presented_as_an_artist_or_album() {
        assert!(is_unknown(" Unknown Artist "));
        assert!(artist_names("Unknown, n/a, Akon").into_iter().eq(["Akon"]));
        assert_eq!(display_metadata("Unknown"), "—");
    }

    #[test]
    fn album_groups_combine_credits_by_album_title() {
        let tracks = [
            Track {
                album: "Freedom".into(),
                artist: "Akon, Unknown".into(),
                ..Track::default()
            },
            Track {
                album: "Freedom".into(),
                artist: "Wyclef Jean".into(),
                ..Track::default()
            },
        ];
        let app = PlayerApp {
            tracks: tracks.to_vec(),
            ..PlayerApp::new()
        };
        let groups = app.albums(&[0, 1]);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].0, "Freedom");
        assert_eq!(
            merge_credits(tracks.iter().map(|track| track.artist.as_str())),
            ["Akon", "Wyclef Jean"]
        );
    }

    #[test]
    fn older_settings_files_load_with_defaults_for_new_features() {
        let preferences: Preferences =
            serde_json::from_str(r#"{"folders":["C:/Music"]}"#).expect("old settings parse");
        assert_eq!(preferences.folders, [PathBuf::from("C:/Music")]);
        assert!(preferences.liked_songs.is_empty());
        assert!(preferences.playlists.is_empty());
        assert!(preferences.repeat == RepeatMode::Off);
        assert_eq!(preferences.ui_scale, 100.0);
        assert_eq!(preferences.volume, 1.0);
        assert!(preferences.album_view_mode == ViewMode::List);
        assert!(preferences.artist_view_mode == ViewMode::List);
        assert!(preferences.prefer_unplayed);
        assert_eq!(preferences.recommendation_variety, 0.65);
        assert_eq!(preferences.recommendation_size, 40);
    }

    #[test]
    fn album_and_artist_views_round_trip_in_settings() {
        let preferences = Preferences {
            album_view_mode: ViewMode::Grid,
            artist_view_mode: ViewMode::Details,
            ..Preferences::default()
        };
        let encoded = serde_json::to_string(&preferences).expect("settings serialize");
        let decoded: Preferences = serde_json::from_str(&encoded).expect("settings deserialize");
        assert!(decoded.album_view_mode == ViewMode::Grid);
        assert!(decoded.artist_view_mode == ViewMode::Details);
    }

    #[test]
    fn shuffle_keeps_every_queue_item_exactly_once() {
        let mut items = (0..100).collect::<Vec<_>>();
        shuffle_slice(&mut items);
        items.sort_unstable();
        assert_eq!(items, (0..100).collect::<Vec<_>>());
    }

    #[test]
    fn queueing_collection_next_keeps_collection_order() {
        let mut app = PlayerApp::new();
        app.tracks = (0..5)
            .map(|index| Track {
                path: PathBuf::from(format!("{index}.mp3")),
                title: index.to_string(),
                ..Track::default()
            })
            .collect();
        app.queue = vec![0, 1, 2];
        app.current = Some(0);

        app.queue_tracks_next(&[3, 4]);

        assert_eq!(app.queue, [0, 3, 4, 1, 2]);
        assert_eq!(app.current, Some(0));
    }

    #[test]
    fn recommendations_prioritize_unplayed_and_exclude_disliked_tracks() {
        let played = PathBuf::from("played.mp3");
        let unplayed = PathBuf::from("unplayed.mp3");
        let disliked = PathBuf::from("disliked.mp3");
        let mut app = PlayerApp::new();
        app.tracks = vec![
            Track {
                path: played.clone(),
                artist: "Favorite Artist".into(),
                genre: "Pop".into(),
                ..Track::default()
            },
            Track {
                path: unplayed.clone(),
                artist: "Favorite Artist".into(),
                genre: "Pop".into(),
                ..Track::default()
            },
            Track {
                path: disliked.clone(),
                artist: "Favorite Artist".into(),
                genre: "Pop".into(),
                ..Track::default()
            },
        ];
        app.history = vec![played];
        app.disliked_songs.insert(disliked);

        let recommendations = app.compute_recommendations();
        assert_eq!(recommendations.first(), Some(&1));
        assert!(!recommendations.contains(&2));
    }

    #[test]
    fn recommendation_settings_round_trip() {
        let preferences = Preferences {
            discovery: 0.8,
            prefer_unplayed: false,
            recommendation_variety: 0.25,
            recommendation_size: 80,
            ..Preferences::default()
        };
        let encoded = serde_json::to_string(&preferences).expect("settings serialize");
        let decoded: Preferences = serde_json::from_str(&encoded).expect("settings deserialize");

        assert_eq!(decoded.discovery, 0.8);
        assert!(!decoded.prefer_unplayed);
        assert_eq!(decoded.recommendation_variety, 0.25);
        assert_eq!(decoded.recommendation_size, 80);
    }

    #[test]
    fn seek_track_maps_clicks_and_drags_to_song_time() {
        assert_eq!(
            seek_position_from_pointer(100.0, 100.0, 400.0, 240.0),
            Some(0.0)
        );
        assert_eq!(
            seek_position_from_pointer(300.0, 100.0, 400.0, 240.0),
            Some(120.0)
        );
        assert_eq!(
            seek_position_from_pointer(500.0, 100.0, 400.0, 240.0),
            Some(240.0)
        );
    }

    #[test]
    fn seek_track_clamps_pointer_and_rejects_invalid_ranges() {
        assert_eq!(
            seek_position_from_pointer(50.0, 100.0, 400.0, 240.0),
            Some(0.0)
        );
        assert_eq!(
            seek_position_from_pointer(600.0, 100.0, 400.0, 240.0),
            Some(240.0)
        );
        assert_eq!(seek_position_from_pointer(100.0, 100.0, 0.0, 240.0), None);
        assert_eq!(seek_position_from_pointer(100.0, 100.0, 400.0, 0.0), None);
    }

    #[test]
    fn wav_decoder_resumes_from_requested_seek_time() {
        let samples = (0..1000_u16)
            .map(|sample| sample as i16)
            .collect::<Vec<_>>();
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36_u32 + samples.len() as u32 * 2).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16_u32.to_le_bytes());
        wav.extend_from_slice(&1_u16.to_le_bytes());
        wav.extend_from_slice(&1_u16.to_le_bytes());
        wav.extend_from_slice(&1000_u32.to_le_bytes());
        wav.extend_from_slice(&2000_u32.to_le_bytes());
        wav.extend_from_slice(&2_u16.to_le_bytes());
        wav.extend_from_slice(&16_u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(samples.len() as u32 * 2).to_le_bytes());
        for sample in samples {
            wav.extend_from_slice(&sample.to_le_bytes());
        }

        let mut decoder = Decoder::new(Cursor::new(wav)).expect("decode test WAV");
        decoder
            .try_seek(Duration::from_millis(500))
            .expect("seek test WAV");
        let sample = decoder.next().expect("sample after seek");
        assert!((499..=501).contains(&sample));
    }
}

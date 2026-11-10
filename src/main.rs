#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

// Lilt source — refreshed attachment revision
use std::{
    collections::{hash_map::DefaultHasher, BTreeMap, HashMap, HashSet, VecDeque},
    fs::{self, File},
    hash::{Hash, Hasher},
    io::{BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, Receiver, Sender, TryRecvError},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use directories::ProjectDirs;
use eframe::egui::{self, Color32, RichText, TextureHandle, Vec2};
use lofty::{prelude::*, probe::Probe};
use rodio::{Decoder, OutputStream, Sink, Source};
use serde::{Deserialize, Serialize};
use souvlaki::{MediaControlEvent, MediaControls, PlatformConfig};

const APP_NAME: &str = "Lilt";
// Large embedded cover images are skipped to prevent libraries with huge art
// from retaining gigabytes of duplicated image data in RAM.
const MAX_CACHED_ARTWORK_BYTES: usize = 2 * 1024 * 1024;
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
    artwork: Option<Arc<Vec<u8>>>,
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

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum NowPlayingTab {
    #[default]
    UpNext,
    ForYou,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct DeletedSongRecord {
    // FNV-1a 128 over the complete file contents. None means the file could not be read
    // when it was deleted; such an entry is reviewable but cannot trigger auto-removal.
    #[serde(default)]
    fingerprint: Option<String>,
    #[serde(default)]
    size_bytes: u64,
    title: String,
    artist: String,
    album: String,
    original_path: PathBuf,
    deleted_at: u64,
    reason: String,
    #[serde(default)]
    preserved_paths: Vec<PathBuf>,
}

#[derive(Clone)]
struct DeletedFingerprintRule {
    fingerprint: String,
    size_bytes: u64,
    preserved_paths: HashSet<PathBuf>,
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
    #[serde(default = "default_duplicate_scan_enabled")]
    duplicate_scan_enabled: bool,
    #[serde(default = "default_duplicate_check_exact")]
    duplicate_check_exact: bool,
    #[serde(default = "default_duplicate_check_recordings")]
    duplicate_check_recordings: bool,
    #[serde(default)]
    duplicate_check_partial: bool,
    #[serde(default)]
    deleted_songs: Vec<DeletedSongRecord>,
    #[serde(default = "default_auto_purge_deleted")]
    auto_purge_deleted: bool,
    #[serde(default = "default_scan_rate_per_second")]
    scan_rate_per_second: usize,
    #[serde(default)]
    daily_mixes: Vec<Vec<PathBuf>>,
    #[serde(default)]
    show_queue_on_start: bool,
    #[serde(default = "default_compact_mode")]
    compact_mode: bool,
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
            duplicate_scan_enabled: default_duplicate_scan_enabled(),
            duplicate_check_exact: default_duplicate_check_exact(),
            duplicate_check_recordings: default_duplicate_check_recordings(),
            duplicate_check_partial: false,
            deleted_songs: Vec::new(),
            auto_purge_deleted: default_auto_purge_deleted(),
            scan_rate_per_second: default_scan_rate_per_second(),
            daily_mixes: Vec::new(),
            show_queue_on_start: false,
            compact_mode: default_compact_mode(),
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

fn default_duplicate_scan_enabled() -> bool {
    true
}

fn default_duplicate_check_exact() -> bool {
    true
}

fn default_duplicate_check_recordings() -> bool {
    true
}

fn default_auto_purge_deleted() -> bool {
    true
}

fn default_scan_rate_per_second() -> usize {
    5_000
}

fn default_compact_mode() -> bool {
    true
}

fn request_repaint_when_focused(context: &egui::Context) {
    if context.input(|input| input.focused) {
        context.request_repaint();
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Home,
    Discover,
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
    auto_deleted_count: usize,
    auto_delete_failures: usize,
    auto_deleted_paths: Vec<PathBuf>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DuplicateKind {
    BitForBit,
    SameRecording,
    PartialMatch,
}

impl DuplicateKind {
    fn title(self) -> &'static str {
        match self {
            Self::BitForBit => "Bit-for-bit duplicates",
            Self::SameRecording => "Same song · alternate file",
            Self::PartialMatch => "Possible partial matches",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::BitForBit => "The audio files have identical contents. One copy is safe to keep.",
            Self::SameRecording => "Title, artist and runtime match, but the files differ (often a different encode or format). Review before deleting.",
            Self::PartialMatch => "Title and runtime are similar, but artist metadata differs or is missing. These may be different recordings.",
        }
    }
}

#[derive(Clone, Debug)]
struct DuplicateCandidate {
    path: PathBuf,
    title: String,
    artist: String,
    duration: f64,
}

#[derive(Clone, Debug)]
struct DuplicateTrack {
    path: PathBuf,
    title: String,
    artist: String,
    duration: f64,
    size_bytes: u64,
}

#[derive(Clone, Debug)]
struct DuplicateGroup {
    kind: DuplicateKind,
    tracks: Vec<DuplicateTrack>,
}

struct DuplicateScanResult {
    groups: Vec<DuplicateGroup>,
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

// Result of the Home background worker. `mixes` is only present when the user
// explicitly pressed Refresh (or on the very first run when none exist yet).
struct HomeJobResult {
    revision: u64,
    recommendations: Option<Vec<usize>>,
    mixes: Option<[Vec<PathBuf>; 3]>,
    seed: u64,
}

// Owned snapshot of everything the recommendation / mix scoring needs, so the
// work can run on a background thread without touching the UI state.
struct MixContext {
    tracks: Vec<Track>,
    history: Vec<PathBuf>,
    liked_songs: HashSet<PathBuf>,
    disliked_songs: HashSet<PathBuf>,
    excluded_tracks: HashSet<PathBuf>,
    liked_artists: HashSet<String>,
    liked_albums: HashSet<String>,
    discovery: f32,
    prefer_unplayed: bool,
    recommendation_variety: f32,
    recommendation_size: usize,
    home_genre: String,
}

#[derive(Clone, Debug, Default)]
struct FileDetails {
    size_bytes: Option<u64>,
    format: Option<String>,
    bitrate_kbps: Option<u32>,
    sample_rate_hz: Option<u32>,
    channels: Option<u8>,
    bit_depth: Option<u8>,
    modified: Option<u64>,
    error: Option<String>,
}

struct SongPropertiesView {
    path: PathBuf,
    title: String,
    artist: String,
    album: String,
    genre: String,
    year: u32,
    duration: f64,
    details: Option<FileDetails>,
}

struct HomeCard {
    title: String,
    subtitle: String,
    cover_index: usize,
    tracks: Vec<usize>,
    start: usize,
}

// Home's card lists are expensive to derive from a large library. Keep them
// between frames and rebuild only after library/listening/reaction changes.
#[derive(Default)]
struct HomeViewCache {
    genres: Vec<(String, usize)>,
    daily_mix_cards: Vec<HomeCard>,
    recent_cards: Vec<HomeCard>,
    mixes: Vec<HomeCard>,
    unplayed_cards: Vec<HomeCard>,
    genre_mix_cards: Vec<HomeCard>,
    decade_mix_cards: Vec<HomeCard>,
    library_cards: Vec<HomeCard>,
    latest_cards: Vec<HomeCard>,
    welcome_resume: Option<(usize, String)>,
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
    daily_mixes: [Vec<usize>; 3],
    daily_mix_paths: [Vec<PathBuf>; 3],
    daily_mix_seed: u64,
    home_job_receiver: Option<Receiver<HomeJobResult>>,
    home_data_revision: u64,
    mix_refresh_requested: bool,
    mix_refresh_running: bool,
    initial_mixes_attempted: bool,
    properties_view: Option<SongPropertiesView>,
    properties_receiver: Option<Receiver<(PathBuf, FileDetails)>>,
    latest_tracks: Vec<usize>,
    home_albums: Vec<(String, Vec<usize>)>,
    home_shelf_offsets: HashMap<String, f32>,
    home_cache: Option<HomeViewCache>,
    home_cache_dirty: bool,
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
    pending_bulk_delete: Option<(Vec<PathBuf>, String)>,
    deleted_songs: Vec<DeletedSongRecord>,
    auto_purge_deleted: bool,
    clear_deleted_history_confirmation: bool,
    lyrics: Option<String>,
    lyrics_error: Option<String>,
    queue: Vec<usize>,
    queue_collapsed: bool,
    now_playing_expanded: bool,
    now_playing_tab: NowPlayingTab,
    compact_mode: bool,
    current: Option<usize>,
    playback_offset: Duration,
    sink: Option<Sink>,
    stream: Option<OutputStream>,
    scan_receiver: Option<Receiver<ScanResult>>,
    scan_status: String,
    scan_busy: bool,
    initial_library_loading: bool,
    scan_rate_per_second: usize,
    scan_progress: Option<Arc<AtomicUsize>>,
    duplicate_scan_enabled: bool,
    duplicate_check_exact: bool,
    duplicate_check_recordings: bool,
    duplicate_check_partial: bool,
    duplicate_scan_receiver: Option<Receiver<DuplicateScanResult>>,
    duplicate_scan_cancel: Option<Arc<AtomicBool>>,
    duplicate_scan_busy: bool,
    duplicate_groups: Vec<DuplicateGroup>,
    duplicate_notice: bool,
    duplicate_review_open: bool,
    pending_duplicate_delete: Option<Vec<PathBuf>>,
    status: String,
    textures: BTreeMap<PathBuf, TextureHandle>,
    ui_icons: BTreeMap<&'static str, TextureHandle>,
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
            daily_mixes: std::array::from_fn(|_| Vec::new()),
            daily_mix_paths: {
                let mut saved: [Vec<PathBuf>; 3] = std::array::from_fn(|_| Vec::new());
                for (slot, paths) in saved.iter_mut().zip(preferences.daily_mixes) {
                    *slot = paths;
                }
                saved
            },
            daily_mix_seed: 0,
            home_job_receiver: None,
            home_data_revision: 0,
            mix_refresh_requested: false,
            mix_refresh_running: false,
            initial_mixes_attempted: false,
            properties_view: None,
            properties_receiver: None,
            latest_tracks: Vec::new(),
            home_albums: Vec::new(),
            home_shelf_offsets: HashMap::new(),
            home_cache: None,
            home_cache_dirty: true,
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
            pending_bulk_delete: None,
            deleted_songs: preferences.deleted_songs,
            auto_purge_deleted: preferences.auto_purge_deleted,
            clear_deleted_history_confirmation: false,
            lyrics: None,
            lyrics_error: None,
            queue: Vec::new(),
            queue_collapsed: !preferences.show_queue_on_start,
            now_playing_expanded: false,
            now_playing_tab: NowPlayingTab::UpNext,
            compact_mode: preferences.compact_mode,
            current: None,
            playback_offset: Duration::ZERO,
            sink: None,
            stream: None,
            scan_receiver: None,
            scan_status: String::new(),
            scan_busy: false,
            initial_library_loading: false,
            scan_rate_per_second: preferences.scan_rate_per_second.min(100_000),
            scan_progress: None,
            duplicate_scan_enabled: preferences.duplicate_scan_enabled,
            duplicate_check_exact: preferences.duplicate_check_exact,
            duplicate_check_recordings: preferences.duplicate_check_recordings,
            duplicate_check_partial: preferences.duplicate_check_partial,
            duplicate_scan_receiver: None,
            duplicate_scan_cancel: None,
            duplicate_scan_busy: false,
            duplicate_groups: Vec::new(),
            duplicate_notice: false,
            duplicate_review_open: false,
            pending_duplicate_delete: None,
            status: preference_error
                .unwrap_or_else(|| "Choose a music folder to get started.".into()),
            textures: BTreeMap::new(),
            ui_icons: BTreeMap::new(),
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
            app.initial_library_loading = true;
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
        ctx: &egui::Context,
        offset_y: f32,
        viewport_height: f32,
        total_items: usize,
        loaded_items: usize,
        batch_size: usize,
        content_extent: f32,
        near_end_distance: f32,
    ) {
        if loaded_items >= total_items {
            return;
        }

        // Prefetch at least one and a half viewports before the current end.
        // The old two-row trigger could wait until the user was almost at the
        // bottom of a batch, making the scroll area feel like it hit a wall.
        // Use ScrollAreaOutput::content_size so the estimate includes egui's
        // per-row spacing instead of guessing from row_height alone.
        let viewport_height = if viewport_height.is_finite() {
            viewport_height.max(0.0)
        } else {
            0.0
        };
        let offset_y = if offset_y.is_finite() {
            offset_y.max(0.0)
        } else {
            0.0
        };
        let content_extent = if content_extent.is_finite() {
            content_extent.max(0.0)
        } else {
            0.0
        };
        let prefetch_distance = near_end_distance.max(viewport_height * 1.5);
        let visible_end = offset_y + viewport_height;

        if visible_end >= (content_extent - prefetch_distance).max(0.0) {
            let next_loaded = self
                .loaded_collection_items
                .saturating_add(batch_size)
                .min(total_items);
            if next_loaded > self.loaded_collection_items {
                self.loaded_collection_items = next_loaded;
                // The ScrollArea has already laid out this frame. Explicitly
                // schedule another frame so the newly loaded rows appear
                // immediately, without requiring an extra wheel/trackpad tick.
                request_repaint_when_focused(ctx);
            }
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
                    request_repaint_when_focused(&repaint_context);
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
        let base_name = if name.starts_with("My Mix ") || name.starts_with("Daily Mix") {
            name.trim().to_string()
        } else {
            format!("{name} Mix")
        };
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

    fn play_shuffled_indices(&mut self, mut tracks: Vec<usize>) {
        shuffle_slice(&mut tracks);
        self.play_indices(tracks, 0);
    }

    fn play_radio_from_track(&mut self, index: usize) {
        let Some(seed) = self.tracks.get(index).cloned() else {
            return;
        };
        let seed_artists = artist_names(&seed.artist)
            .into_iter()
            .map(|artist| artist.to_lowercase())
            .collect::<HashSet<_>>();
        let mut radio = self
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, track)| {
                !self.excluded_tracks.contains(&track.path)
                    && !self.disliked_songs.contains(&track.path)
                    && (artist_names(&track.artist)
                        .iter()
                        .any(|artist| seed_artists.contains(&artist.to_lowercase()))
                        || (!is_unknown(&seed.genre)
                            && track.genre.eq_ignore_ascii_case(&seed.genre)))
            })
            .map(|(track_index, _)| track_index)
            .collect::<Vec<_>>();
        if !radio.contains(&index) {
            radio.insert(0, index);
        }
        let fallback = self.recommendation_indices();
        for candidate in fallback {
            if radio.len() >= self.recommendation_size.clamp(20, 100) {
                break;
            }
            if !radio.contains(&candidate) {
                radio.push(candidate);
            }
        }
        radio.truncate(self.recommendation_size.clamp(20, 100));
        shuffle_slice(&mut radio);
        self.play_indices(radio, 0);
        self.status = format!("Starting a radio mix around {}.", seed.title);
    }

    fn play_radio_for_artist(&mut self, artist: &str) {
        let mut radio = self
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, track)| {
                !self.excluded_tracks.contains(&track.path)
                    && !self.disliked_songs.contains(&track.path)
                    && artist_names(&track.artist)
                        .iter()
                        .any(|name| name.eq_ignore_ascii_case(artist))
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        let fallback = self.recommendation_indices();
        for candidate in fallback {
            if radio.len() >= self.recommendation_size.clamp(20, 100) {
                break;
            }
            if !radio.contains(&candidate) {
                radio.push(candidate);
            }
        }
        shuffle_slice(&mut radio);
        self.play_indices(radio, 0);
        self.status = format!("Starting {artist} radio.");
    }

    fn show_track_context_menu(&mut self, response: &egui::Response, index: usize) {
        if index >= self.tracks.len() {
            return;
        }
        let mut play_now = false;
        let mut play_next = false;
        let mut enqueue = false;
        let mut start_radio = false;
        let mut toggle_like = false;
        let mut toggle_dislike = false;
        let mut open_album = false;
        let mut open_artist = false;
        let mut create_playlist = false;
        let mut add_to_playlist = None;
        let mut remove_from_playlists = false;
        let mut clear_history = false;
        let mut remove_from_library = false;
        let mut delete_file = false;
        let mut show_properties = false;
        response.context_menu(|ui| {
            // Everything below only runs while the menu is actually open.
            let Some(track) = self.tracks.get(index) else {
                return;
            };
            let title = track.title.clone();
            let path = track.path.clone();
            let album = track.album.clone();
            let artist_credit = track.artist.clone();
            let artist = artist_names(&track.artist).into_iter().next();
            let liked = self.liked_songs.contains(&path);
            let disliked = self.disliked_songs.contains(&path);
            let playlist_names = self
                .playlists
                .iter()
                .map(|playlist| playlist.name.clone())
                .collect::<Vec<_>>();
            ui.set_min_width(250.0);
            ui.label(RichText::new(&title).strong().size(15.0));
            ui.label(
                RichText::new(display_credit(&artist_credit))
                    .small()
                    .color(Color32::GRAY),
            );
            ui.separator();
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
            if ui.button("◉  Start song radio").clicked() {
                start_radio = true;
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
            if ui.button("▤  Copy song details").clicked() {
                ui.ctx().copy_text(format!(
                    "{} — {}\nAlbum: {}\n{}",
                    title,
                    display_credit(&artist_credit),
                    display_metadata(&album),
                    path.display()
                ));
                ui.close_menu();
            }
            if ui.button("▣  Copy file path").clicked() {
                ui.ctx().copy_text(path.to_string_lossy().into_owned());
                ui.close_menu();
            }
            if ui.button("↗  Reveal file in Explorer").clicked() {
                #[cfg(target_os = "windows")]
                {
                    let _ = std::process::Command::new("explorer.exe")
                        .arg(format!("/select,{}", path.display()))
                        .spawn();
                }
                ui.close_menu();
            }
            if ui.button("◷  Clear this song from history").clicked() {
                clear_history = true;
                ui.close_menu();
            }
            if ui.button("−  Remove from all playlists").clicked() {
                remove_from_playlists = true;
                ui.close_menu();
            }
            ui.separator();
            if ui.button("Remove from library (keep file)").clicked() {
                remove_from_library = true;
                ui.close_menu();
            }
            if ui
                .button(RichText::new("Delete audio file permanently…").color(Color32::LIGHT_RED))
                .clicked()
            {
                delete_file = true;
                ui.close_menu();
            }
            ui.separator();
            if ui.button("≡  Properties").clicked() {
                show_properties = true;
                ui.close_menu();
            }
        });
        let any_action = play_now
            || play_next
            || enqueue
            || start_radio
            || toggle_like
            || toggle_dislike
            || open_album
            || open_artist
            || create_playlist
            || add_to_playlist.is_some()
            || remove_from_playlists
            || clear_history
            || remove_from_library
            || delete_file
            || show_properties;
        if !any_action {
            return;
        }
        if show_properties {
            self.open_song_properties(index, &response.ctx);
        }
        let Some(track) = self.tracks.get(index) else {
            return;
        };
        let title = track.title.clone();
        let path = track.path.clone();
        let album = track.album.clone();
        let artist = artist_names(&track.artist).into_iter().next();
        if play_now {
            self.play_indices(vec![index], 0);
        }
        if play_next {
            self.queue_track_next(index);
        }
        if enqueue {
            self.add_tracks_to_queue(&[index]);
        }
        if start_radio {
            self.play_radio_from_track(index);
        }
        if toggle_like {
            self.toggle_song_reaction(index, true);
        }
        if toggle_dislike {
            self.toggle_song_reaction(index, false);
        }
        if open_album {
            self.selected_album = Some(album.clone());
            self.page = Page::AlbumDetail;
        }
        if open_artist {
            self.selected_artist = artist.clone();
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
            if let Some(name) = added_to {
                self.save_state();
                self.status = format!("Added {title} to {name}.");
            }
        }
        if remove_from_playlists {
            for playlist in &mut self.playlists {
                playlist.tracks.retain(|entry| entry != &path);
            }
            self.save_state();
            self.status = format!("Removed {title} from all playlists.");
        }
        if clear_history {
            self.history.retain(|entry| entry != &path);
            self.home_cache_dirty = true;
            self.save_state();
            self.status = format!("Cleared {title} from listening history.");
        }
        if remove_from_library {
            self.excluded_tracks.insert(path.clone());
            self.recommendations_dirty = true;
            self.home_cache_dirty = true;
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
        let mut play = false;
        let mut shuffle = false;
        let mut enqueue = false;
        let mut toggle_saved = false;
        let mut create_playlist = false;
        let mut open = false;
        let mut add_to_playlist = None;
        let mut remove_from_library = false;
        let mut delete_all_files = false;
        let mut clear_history = false;
        let mut remove_from_playlists = false;
        response.context_menu(|ui| {
            let saved = self.liked_albums.contains(&album.to_lowercase());
            let playlist_names = self
                .playlists
                .iter()
                .map(|playlist| playlist.name.clone())
                .collect::<Vec<_>>();
            let credits =
                merge_credits(tracks.iter().filter_map(|&index| {
                    self.tracks.get(index).map(|track| track.artist.as_str())
                }));
            ui.set_min_width(260.0);
            ui.label(RichText::new(&album).strong().size(15.0));
            ui.label(
                RichText::new(format!("{} · {} tracks", credits.join(", "), tracks.len()))
                    .small()
                    .color(Color32::GRAY),
            );
            ui.separator();
            if ui.button("▶  Play album").clicked() {
                play = true;
                ui.close_menu();
            }
            if ui.button("🔀  Shuffle album").clicked() {
                shuffle = true;
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
            if !playlist_names.is_empty() {
                ui.menu_button("＋  Add album to playlist", |ui| {
                    for (playlist_index, name) in playlist_names.iter().enumerate() {
                        if ui.button(name).clicked() {
                            add_to_playlist = Some(playlist_index);
                            ui.close_menu();
                        }
                    }
                });
            }
            if ui.button("＋  Create playlist from album").clicked() {
                create_playlist = true;
                ui.close_menu();
            }
            ui.separator();
            if ui.button("▤  Copy album and artist").clicked() {
                ui.ctx()
                    .copy_text(format!("{}\n{}", album, credits.join(", ")));
                ui.close_menu();
            }
            if ui.button("▤  Copy all album file paths").clicked() {
                let listing = tracks
                    .iter()
                    .filter_map(|&index| {
                        self.tracks
                            .get(index)
                            .map(|track| track.path.to_string_lossy().into_owned())
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                ui.ctx().copy_text(listing);
                ui.close_menu();
            }
            ui.separator();
            if ui.button("◷  Clear album listening history").clicked() {
                clear_history = true;
                ui.close_menu();
            }
            if ui.button("−  Remove album from all playlists").clicked() {
                remove_from_playlists = true;
                ui.close_menu();
            }
            if ui
                .button("Remove all album tracks from library (keep files)")
                .clicked()
            {
                remove_from_library = true;
                ui.close_menu();
            }
            if ui
                .button(
                    RichText::new("Delete all album audio files permanently…")
                        .color(Color32::LIGHT_RED),
                )
                .clicked()
            {
                delete_all_files = true;
                ui.close_menu();
            }
        });
        let paths = tracks
            .iter()
            .filter_map(|&index| self.tracks.get(index).map(|track| track.path.clone()))
            .collect::<Vec<_>>();
        if clear_history {
            let path_set = paths.iter().cloned().collect::<HashSet<_>>();
            self.history.retain(|path| !path_set.contains(path));
            self.home_cache_dirty = true;
            self.save_state();
            self.status = format!("Cleared listening history for album {album}.");
        }
        if remove_from_playlists {
            let path_set = paths.iter().cloned().collect::<HashSet<_>>();
            for playlist in &mut self.playlists {
                playlist.tracks.retain(|path| !path_set.contains(path));
            }
            self.save_state();
            self.status = format!("Removed album {album} from all playlists.");
        }
        if remove_from_library {
            let mut added = 0;
            for path in &paths {
                if self.excluded_tracks.insert(path.clone()) {
                    added += 1;
                }
            }
            self.recommendations_dirty = true;
            self.home_cache_dirty = true;
            self.save_state();
            self.status =
                format!("Removed {added} album track(s) from the library. Files were kept.");
        }
        if delete_all_files {
            self.pending_bulk_delete = Some((paths.clone(), format!("album ‘{album}’")));
        }
        if play {
            self.play_all_indices(tracks.clone());
        }
        if shuffle {
            self.play_shuffled_indices(tracks.clone());
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
        if let Some(playlist_index) = add_to_playlist {
            if let Some(playlist) = self.playlists.get_mut(playlist_index) {
                let mut added = 0;
                for &index in &tracks {
                    if let Some(path) = self.tracks.get(index).map(|track| track.path.clone()) {
                        if !playlist.tracks.contains(&path) {
                            playlist.tracks.push(path);
                            added += 1;
                        }
                    }
                }
                let name = playlist.name.clone();
                self.save_state();
                self.status = format!("Added {added} album tracks to {name}.");
            }
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
        let mut play = false;
        let mut shuffle = false;
        let mut enqueue = false;
        let mut radio = false;
        let mut toggle_follow = false;
        let mut create_playlist = false;
        let mut open = false;
        let mut add_to_playlist = None;
        let mut remove_from_library = false;
        let mut delete_all_files = false;
        let mut clear_history = false;
        let mut remove_from_playlists = false;
        response.context_menu(|ui| {
            let followed = self.liked_artists.contains(&artist.to_lowercase());
            let playlist_names = self
                .playlists
                .iter()
                .map(|playlist| playlist.name.clone())
                .collect::<Vec<_>>();
            ui.set_min_width(260.0);
            ui.label(RichText::new(&artist).strong().size(15.0));
            ui.label(
                RichText::new(format!("{} tracks in your library", tracks.len()))
                    .small()
                    .color(Color32::GRAY),
            );
            ui.separator();
            if ui.button("▶  Play artist").clicked() {
                play = true;
                ui.close_menu();
            }
            if ui.button("🔀  Shuffle artist").clicked() {
                shuffle = true;
                ui.close_menu();
            }
            if ui.button("⏭  Play artist next").clicked() {
                enqueue = true;
                ui.close_menu();
            }
            if ui.button("◉  Start artist radio").clicked() {
                radio = true;
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
            if !playlist_names.is_empty() {
                ui.menu_button("＋  Add artist to playlist", |ui| {
                    for (playlist_index, name) in playlist_names.iter().enumerate() {
                        if ui.button(name).clicked() {
                            add_to_playlist = Some(playlist_index);
                            ui.close_menu();
                        }
                    }
                });
            }
            if ui.button("＋  Create playlist from artist").clicked() {
                create_playlist = true;
                ui.close_menu();
            }
            ui.separator();
            if ui.button("▤  Copy artist name").clicked() {
                ui.ctx().copy_text(artist.clone());
                ui.close_menu();
            }
            if ui.button("▤  Copy all artist file paths").clicked() {
                let listing = tracks
                    .iter()
                    .filter_map(|&index| {
                        self.tracks
                            .get(index)
                            .map(|track| track.path.to_string_lossy().into_owned())
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                ui.ctx().copy_text(listing);
                ui.close_menu();
            }
            ui.separator();
            if ui.button("◷  Clear artist listening history").clicked() {
                clear_history = true;
                ui.close_menu();
            }
            if ui.button("−  Remove artist from all playlists").clicked() {
                remove_from_playlists = true;
                ui.close_menu();
            }
            if ui
                .button("Remove all artist tracks from library (keep files)")
                .clicked()
            {
                remove_from_library = true;
                ui.close_menu();
            }
            if ui
                .button(
                    RichText::new("Delete all artist audio files permanently…")
                        .color(Color32::LIGHT_RED),
                )
                .clicked()
            {
                delete_all_files = true;
                ui.close_menu();
            }
        });
        let paths = tracks
            .iter()
            .filter_map(|&index| self.tracks.get(index).map(|track| track.path.clone()))
            .collect::<Vec<_>>();
        let path_set = paths.iter().cloned().collect::<HashSet<_>>();
        if clear_history {
            self.history.retain(|path| !path_set.contains(path));
            self.home_cache_dirty = true;
            self.save_state();
            self.status = format!("Cleared listening history for {artist}.");
        }
        if remove_from_playlists {
            for playlist in &mut self.playlists {
                playlist.tracks.retain(|path| !path_set.contains(path));
            }
            self.save_state();
            self.status = format!("Removed {artist} from all playlists.");
        }
        if remove_from_library {
            let mut added = 0;
            for path in &paths {
                if self.excluded_tracks.insert(path.clone()) {
                    added += 1;
                }
            }
            self.recommendations_dirty = true;
            self.home_cache_dirty = true;
            self.save_state();
            self.status =
                format!("Removed {added} track(s) by {artist} from the library. Files were kept.");
        }
        if delete_all_files {
            self.pending_bulk_delete = Some((paths.clone(), format!("artist ‘{artist}’")));
        }
        if play {
            self.play_all_indices(tracks.clone());
        }
        if shuffle {
            self.play_shuffled_indices(tracks.clone());
        }
        if enqueue {
            self.queue_tracks_next(&tracks);
        }
        if radio {
            self.play_radio_for_artist(&artist);
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
        if let Some(playlist_index) = add_to_playlist {
            if let Some(playlist) = self.playlists.get_mut(playlist_index) {
                let mut added = 0;
                for &index in &tracks {
                    if let Some(path) = self.tracks.get(index).map(|track| track.path.clone()) {
                        if !playlist.tracks.contains(&path) {
                            playlist.tracks.push(path);
                            added += 1;
                        }
                    }
                }
                let name = playlist.name.clone();
                self.save_state();
                self.status = format!("Added {added} artist tracks to {name}.");
            }
        }
    }

    fn show_home_card_context_menu(&mut self, response: &egui::Response, card: &HomeCard) {
        let mut play = false;
        let mut shuffle = false;
        let mut enqueue = false;
        let mut create_playlist = false;
        let mut remove_from_library = false;
        let mut delete_all_files = false;
        let mut clear_history = false;
        response.context_menu(|ui| {
            ui.set_min_width(270.0);
            ui.label(RichText::new(&card.title).strong());
            ui.label(
                RichText::new(format!("{} tracks", card.tracks.len()))
                    .small()
                    .color(Color32::GRAY),
            );
            ui.separator();
            if ui.button("▶  Play from here").clicked() {
                play = true;
                ui.close_menu();
            }
            if ui.button("🔀  Shuffle and play").clicked() {
                shuffle = true;
                ui.close_menu();
            }
            if ui.button("♫  Add mix to queue").clicked() {
                enqueue = true;
                ui.close_menu();
            }
            if ui.button("＋  Save mix as playlist").clicked() {
                create_playlist = true;
                ui.close_menu();
            }
            ui.separator();
            if ui.button("▤  Copy mix track list").clicked() {
                let listing = card
                    .tracks
                    .iter()
                    .filter_map(|&index| {
                        self.tracks.get(index).map(|track| {
                            format!("{} — {}", track.title, display_credit(&track.artist))
                        })
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                ui.ctx().copy_text(listing);
                ui.close_menu();
            }
            ui.separator();
            if ui.button("◷  Clear these tracks from history").clicked() {
                clear_history = true;
                ui.close_menu();
            }
            if ui
                .button("Remove all mix tracks from library (keep files)")
                .clicked()
            {
                remove_from_library = true;
                ui.close_menu();
            }
            if ui
                .button(
                    RichText::new("Delete all mix audio files permanently…")
                        .color(Color32::LIGHT_RED),
                )
                .clicked()
            {
                delete_all_files = true;
                ui.close_menu();
            }
        });
        if !(play
            || shuffle
            || enqueue
            || create_playlist
            || remove_from_library
            || delete_all_files
            || clear_history)
        {
            return;
        }
        let title = card.title.clone();
        let tracks = card.tracks.clone();
        let start = card.start;
        let paths = tracks
            .iter()
            .filter_map(|&index| self.tracks.get(index).map(|track| track.path.clone()))
            .collect::<Vec<_>>();
        let path_set = paths.iter().cloned().collect::<HashSet<_>>();
        if clear_history {
            self.history.retain(|path| !path_set.contains(path));
            self.home_cache_dirty = true;
            self.save_state();
            self.status = format!("Cleared listening history for {title}.");
        }
        if remove_from_library {
            let mut added = 0;
            for path in &paths {
                if self.excluded_tracks.insert(path.clone()) {
                    added += 1;
                }
            }
            self.recommendations_dirty = true;
            self.home_cache_dirty = true;
            self.save_state();
            self.status =
                format!("Removed {added} mix track(s) from the library. Files were kept.");
        }
        if delete_all_files {
            self.pending_bulk_delete = Some((paths.clone(), format!("mix ‘{title}’")));
        }
        if play {
            self.play_indices(tracks.clone(), start);
        }
        if shuffle {
            self.play_shuffled_indices(tracks.clone());
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
        if let Some(cancel) = self.duplicate_scan_cancel.take() {
            cancel.store(true, Ordering::Release);
        }
        self.duplicate_scan_receiver = None;
        self.duplicate_scan_busy = false;
        self.duplicate_groups.clear();
        self.duplicate_notice = false;
        self.duplicate_review_open = false;
        let folders = self.folders.clone();
        let deleted_rules = build_deleted_fingerprint_rules(&self.deleted_songs);
        let auto_purge_deleted = self.auto_purge_deleted;
        let scan_rate_per_second = self.scan_rate_per_second;
        let scan_progress = Arc::new(AtomicUsize::new(0));
        self.scan_progress = Some(Arc::clone(&scan_progress));
        let (sender, receiver) = mpsc::channel();
        self.scan_receiver = Some(receiver);
        self.scan_busy = true;
        self.scan_status = "Scanning folders… 0 songs checked".into();
        self.status = "Scanning music folders…".into();
        thread::spawn(move || {
            let result = scan_folders_with_settings(
                &folders,
                &deleted_rules,
                auto_purge_deleted,
                scan_rate_per_second,
                scan_progress,
            );
            let _ = sender.send(result);
        });
    }

    fn poll_scan(&mut self) {
        if self.scan_busy {
            let scanned = self
                .scan_progress
                .as_ref()
                .map_or(0, |progress| progress.load(Ordering::Relaxed));
            self.scan_status = if self.scan_rate_per_second == 0 {
                format!("Scanning · {scanned} songs checked · unlimited")
            } else {
                format!(
                    "Scanning · {scanned} songs checked · up to {}/s",
                    self.scan_rate_per_second
                )
            };
        }
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
                self.initial_library_loading = false;
                self.scan_receiver = None;
                self.scan_progress = None;
                let auto_deleted_count = result.auto_deleted_count;
                let auto_delete_failures = result.auto_delete_failures;
                let auto_deleted_set = result
                    .auto_deleted_paths
                    .iter()
                    .cloned()
                    .collect::<HashSet<_>>();
                if !auto_deleted_set.is_empty() {
                    self.liked_songs
                        .retain(|path| !auto_deleted_set.contains(path));
                    self.disliked_songs
                        .retain(|path| !auto_deleted_set.contains(path));
                    self.excluded_tracks
                        .retain(|path| !auto_deleted_set.contains(path));
                    self.history.retain(|path| !auto_deleted_set.contains(path));
                    self.home_cache_dirty = true;
                    for playlist in &mut self.playlists {
                        playlist
                            .tracks
                            .retain(|path| !auto_deleted_set.contains(path));
                    }
                }
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
                self.recommendations.clear();
                self.recommendations_dirty = true;
                self.home_data_revision = self.home_data_revision.wrapping_add(1);
                self.home_cache_dirty = true;
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
                if auto_deleted_count > 0 {
                    self.status = format!(
                        "{} · automatically removed {} previously deleted file(s)",
                        self.status, auto_deleted_count
                    );
                }
                if auto_delete_failures > 0 {
                    self.status = format!(
                        "{} · {} matching file(s) could not be removed",
                        self.status, auto_delete_failures
                    );
                }
                if let Some(error) = result.error {
                    self.status = format!("{} · {error}", self.status);
                }
                if !auto_deleted_set.is_empty() {
                    self.save_state();
                }
                self.start_duplicate_scan();
            }
            Err(TryRecvError::Disconnected) => {
                self.scan_busy = false;
                self.initial_library_loading = false;
                self.scan_receiver = None;
                self.scan_progress = None;
                self.scan_status = "Library scan stopped unexpectedly.".into();
            }
            Err(TryRecvError::Empty) => {}
        }
    }

    fn start_duplicate_scan(&mut self) {
        if !self.duplicate_scan_enabled {
            self.duplicate_groups.clear();
            self.duplicate_scan_busy = false;
            self.duplicate_notice = false;
            self.duplicate_scan_receiver = None;
            return;
        }
        if let Some(cancel) = self.duplicate_scan_cancel.take() {
            cancel.store(true, Ordering::Release);
        }
        self.duplicate_groups.clear();
        self.duplicate_notice = false;
        self.duplicate_review_open = false;
        // Liked songs are protected from duplicate suggestions. If the user likes or
        // unlikes a song, toggle_song_reaction restarts this background scan so the
        // duplicate list immediately reflects the updated preference.
        let candidates = self
            .tracks
            .iter()
            .filter(|track| !self.liked_songs.contains(&track.path))
            .map(|track| DuplicateCandidate {
                path: track.path.clone(),
                title: track.title.clone(),
                artist: track.artist.clone(),
                duration: track.duration,
            })
            .collect::<Vec<_>>();
        if candidates.len() < 2 {
            self.duplicate_groups.clear();
            self.duplicate_notice = false;
            self.duplicate_scan_busy = false;
            self.duplicate_scan_receiver = None;
            return;
        }

        let check_exact = self.duplicate_check_exact;
        let check_recordings = self.duplicate_check_recordings;
        let check_partial = self.duplicate_check_partial;
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let (sender, receiver) = mpsc::channel();
        self.duplicate_scan_cancel = Some(cancel);
        self.duplicate_scan_receiver = Some(receiver);
        self.duplicate_scan_busy = true;
        self.duplicate_notice = false;

        thread::spawn(move || {
            let groups = find_duplicate_groups(
                candidates,
                check_exact,
                check_recordings,
                check_partial,
                &worker_cancel,
            );
            if !worker_cancel.load(Ordering::Acquire) {
                let _ = sender.send(DuplicateScanResult { groups });
            }
        });
    }

    fn poll_duplicate_scan(&mut self) {
        let received = match self.duplicate_scan_receiver.as_ref() {
            Some(receiver) => receiver.try_recv(),
            None => return,
        };
        match received {
            Ok(result) => {
                self.duplicate_scan_receiver = None;
                self.duplicate_scan_cancel = None;
                self.duplicate_scan_busy = false;
                self.duplicate_groups = result.groups;
                self.duplicate_notice = !self.duplicate_groups.is_empty();
            }
            Err(TryRecvError::Disconnected) => {
                self.duplicate_scan_receiver = None;
                self.duplicate_scan_cancel = None;
                self.duplicate_scan_busy = false;
            }
            Err(TryRecvError::Empty) => {}
        }
    }

    fn poll_artwork(&mut self, context: &egui::Context) {
        // Upload at most one decoded image per frame. This prevents a burst of
        // completed artwork jobs from stalling the UI on the render thread.
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
                    while self.textures.len() >= 192 {
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
                if !self.artwork_pending.is_empty() && context.input(|input| input.focused) {
                    context.request_repaint_after(Duration::from_millis(35));
                }
            }
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => {}
        }
    }

    fn add_folder(&mut self) {
        if let Some(folder) = rfd::FileDialog::new().pick_folder() {
            if self.tracks.is_empty() {
                self.initial_library_loading = true;
            }
            if !self.folders.contains(&folder) {
                self.folders.push(folder);
                self.save_state();
            }
            self.start_scan();
        }
    }

    fn save_state(&mut self) -> bool {
        let preferences = self.preferences_snapshot();
        match save_preferences(&preferences) {
            Ok(()) => true,
            Err(error) => {
                self.status = error;
                false
            }
        }
    }

    fn make_deleted_song_record(
        &self,
        path: &Path,
        paths_being_deleted: &HashSet<PathBuf>,
        reason: &str,
    ) -> DeletedSongRecord {
        let track = self.tracks.iter().find(|track| track.path == path);
        let size_bytes = fs::metadata(path)
            .map(|metadata| metadata.len())
            .unwrap_or_default();
        let fingerprint = file_content_fingerprint(path).ok();
        let mut preserved_paths = Vec::new();

        // If an identical copy is intentionally being kept, remember its path so
        // auto-cleanup never removes that survivor on the next scan.
        if let Some(expected_fingerprint) = fingerprint.as_deref() {
            for candidate in &self.tracks {
                if candidate.path == path || paths_being_deleted.contains(&candidate.path) {
                    continue;
                }
                let candidate_size = fs::metadata(&candidate.path)
                    .map(|metadata| metadata.len())
                    .unwrap_or_default();
                if candidate_size == size_bytes
                    && file_content_fingerprint(&candidate.path).ok().as_deref()
                        == Some(expected_fingerprint)
                {
                    preserved_paths.push(candidate.path.clone());
                }
            }
        }

        DeletedSongRecord {
            fingerprint,
            size_bytes,
            title: track
                .map(|track| track.title.clone())
                .or_else(|| {
                    path.file_stem()
                        .map(|stem| stem.to_string_lossy().into_owned())
                })
                .unwrap_or_else(|| "Unknown song".into()),
            artist: track.map_or_else(String::new, |track| track.artist.clone()),
            album: track.map_or_else(String::new, |track| track.album.clone()),
            original_path: path.to_path_buf(),
            deleted_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            reason: reason.to_string(),
            preserved_paths,
        }
    }

    fn remember_deleted_song(&mut self, record: DeletedSongRecord) {
        if let Some(fingerprint) = record.fingerprint.as_deref() {
            if let Some(existing) = self.deleted_songs.iter_mut().find(|existing| {
                existing.size_bytes == record.size_bytes
                    && existing.fingerprint.as_deref() == Some(fingerprint)
            }) {
                // A copy that was previously preserved may itself be deleted later.
                // Remove that path from the allow-list before merging current survivors.
                existing
                    .preserved_paths
                    .retain(|path| path != &record.original_path);
                for path in record.preserved_paths {
                    if !existing.preserved_paths.contains(&path) {
                        existing.preserved_paths.push(path);
                    }
                }
                existing.deleted_at = existing.deleted_at.max(record.deleted_at);
                existing.reason = record.reason;
                return;
            }
        } else if self.deleted_songs.iter().any(|existing| {
            existing.fingerprint.is_none() && existing.original_path == record.original_path
        }) {
            return;
        }
        self.deleted_songs.push(record);
    }

    fn allow_deleted_song_again(&mut self, index: usize) {
        if index < self.deleted_songs.len() {
            let removed = self.deleted_songs.remove(index);
            let save_error = if self.save_state() {
                None
            } else {
                Some(self.status.clone())
            };
            self.start_scan();
            self.status = match save_error {
                Some(error) => format!("Rule changed, but settings could not be saved: {error}"),
                None => format!(
                    "Auto-removal rule removed for {}. The file can be added again.",
                    removed.title
                ),
            };
        }
    }

    fn delete_audio_file(&mut self, path: &Path) {
        let Some(track_index) = self.tracks.iter().position(|track| track.path == path) else {
            self.status = "The selected track is no longer in the library.".into();
            return;
        };
        let title = self.tracks[track_index].title.clone();
        let paths_being_deleted = HashSet::from([path.to_path_buf()]);
        let deleted_record =
            self.make_deleted_song_record(path, &paths_being_deleted, "Manually deleted");
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
        self.remember_deleted_song(deleted_record);
        self.liked_songs.remove(path);
        self.disliked_songs.remove(path);
        self.excluded_tracks.remove(path);
        self.history.retain(|entry| entry != path);
        self.home_cache_dirty = true;
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
        let save_error = if self.save_state() {
            None
        } else {
            Some(self.status.clone())
        };
        self.start_scan();
        self.status = match save_error {
            Some(error) => format!(
                "Permanently deleted {title}, but its delete rule could not be saved: {error}"
            ),
            None => format!("Permanently deleted {title}."),
        };
    }

    fn delete_audio_files_permanently(
        &mut self,
        paths: &[PathBuf],
        reason: &str,
        description: &str,
    ) {
        let mut unique_paths = Vec::new();
        let mut paths_being_deleted = HashSet::new();
        for path in paths {
            if paths_being_deleted.insert(path.clone()) {
                unique_paths.push(path.clone());
            }
        }
        if unique_paths.is_empty() {
            self.status = "There are no files to delete.".into();
            return;
        }

        let records = unique_paths
            .iter()
            .map(|path| {
                (
                    path.clone(),
                    self.make_deleted_song_record(path, &paths_being_deleted, reason),
                )
            })
            .collect::<HashMap<_, _>>();

        let current_path = self
            .current
            .and_then(|position| self.queue.get(position))
            .and_then(|&index| self.tracks.get(index))
            .map(|track| track.path.clone());
        let mut deleted = Vec::new();
        let mut failures = Vec::new();
        for path in &unique_paths {
            match fs::remove_file(path) {
                Ok(()) => {
                    deleted.push(path.clone());
                    if let Some(record) = records.get(path).cloned() {
                        self.remember_deleted_song(record);
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => failures.push(format!("{}: {error}", path.display())),
            }
        }

        if deleted.is_empty() {
            self.status = if failures.is_empty() {
                format!("No existing files were found for {description}.")
            } else {
                format!("Could not delete {description}: {}", failures.join("; "))
            };
            return;
        }

        let deleted_set = deleted.iter().cloned().collect::<HashSet<_>>();
        if current_path
            .as_ref()
            .is_some_and(|path| deleted_set.contains(path))
        {
            self.sink = None;
            self.stream = None;
            self.current = None;
            self.playback_offset = Duration::ZERO;
        }
        self.liked_songs.retain(|path| !deleted_set.contains(path));
        self.disliked_songs
            .retain(|path| !deleted_set.contains(path));
        self.excluded_tracks
            .retain(|path| !deleted_set.contains(path));
        self.history.retain(|path| !deleted_set.contains(path));
        self.home_cache_dirty = true;
        for playlist in &mut self.playlists {
            playlist.tracks.retain(|path| !deleted_set.contains(path));
        }
        self.recommendations_dirty = true;
        self.home_cache_dirty = true;
        self.duplicate_groups.clear();
        self.duplicate_notice = false;
        self.duplicate_review_open = false;

        let save_error = if self.save_state() {
            None
        } else {
            Some(self.status.clone())
        };
        self.start_scan();
        self.status = if let Some(error) = save_error {
            format!(
                "Deleted {} file(s) from {description}, but the deletion history could not be saved: {error}",
                deleted.len()
            )
        } else if failures.is_empty() {
            format!(
                "Permanently deleted {} file(s) from {description}.",
                deleted.len()
            )
        } else {
            format!(
                "Deleted {} file(s) from {description}; {} could not be deleted.",
                deleted.len(),
                failures.len()
            )
        };
    }

    fn delete_duplicate_files(&mut self, paths: &[PathBuf]) {
        let mut unique_paths = Vec::new();
        let mut deleted_set = HashSet::new();
        for path in paths {
            if deleted_set.insert(path.clone()) {
                unique_paths.push(path.clone());
            }
        }
        let records = unique_paths
            .iter()
            .map(|path| {
                (
                    path.clone(),
                    self.make_deleted_song_record(path, &deleted_set, "Duplicate cleanup"),
                )
            })
            .collect::<HashMap<_, _>>();

        let mut deleted = Vec::new();
        let mut failures = Vec::new();
        for path in &unique_paths {
            match fs::remove_file(path) {
                Ok(()) => {
                    deleted.push(path.clone());
                    if let Some(record) = records.get(path).cloned() {
                        self.remember_deleted_song(record);
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => failures.push(format!("{}: {error}", path.display())),
            }
        }
        if deleted.is_empty() {
            self.status = if failures.is_empty() {
                "Those duplicate files were already gone.".into()
            } else {
                format!("Could not delete duplicate files: {}", failures.join("; "))
            };
            return;
        }

        let deleted_set = deleted.iter().cloned().collect::<HashSet<_>>();
        let current_path = self
            .current
            .and_then(|position| self.queue.get(position))
            .and_then(|&index| self.tracks.get(index))
            .map(|track| track.path.clone());
        if current_path
            .as_ref()
            .is_some_and(|path| deleted_set.contains(path))
        {
            self.sink = None;
            self.stream = None;
            self.current = None;
        }
        self.liked_songs.retain(|path| !deleted_set.contains(path));
        self.disliked_songs
            .retain(|path| !deleted_set.contains(path));
        self.excluded_tracks
            .retain(|path| !deleted_set.contains(path));
        self.history.retain(|path| !deleted_set.contains(path));
        self.home_cache_dirty = true;
        for playlist in &mut self.playlists {
            playlist.tracks.retain(|path| !deleted_set.contains(path));
        }
        self.duplicate_groups.clear();
        self.duplicate_notice = false;
        self.duplicate_review_open = false;
        let save_error = if self.save_state() {
            None
        } else {
            Some(self.status.clone())
        };
        self.start_scan();
        self.status = if let Some(error) = save_error {
            format!(
                "Deleted {} file(s), but the delete history could not be saved: {error}",
                deleted.len()
            )
        } else if failures.is_empty() {
            format!(
                "Permanently deleted {} exact duplicate file(s).",
                deleted.len()
            )
        } else {
            format!(
                "Deleted {} exact duplicate file(s); {} could not be deleted.",
                deleted.len(),
                failures.len()
            )
        };
    }

    fn remove_duplicate_suggestions_from_library(&mut self, paths: &[PathBuf]) {
        let mut added = 0;
        for path in paths {
            if self.excluded_tracks.insert(path.clone()) {
                added += 1;
            }
        }
        if added == 0 {
            self.status = "Those tracks were already excluded from the library.".into();
            return;
        }
        self.recommendations_dirty = true;
        self.home_cache_dirty = true;
        self.duplicate_groups.clear();
        self.duplicate_notice = false;
        self.save_state();
        self.start_scan();
        self.status = format!(
            "Removed {added} possible duplicate(s) from the library. Their files were kept; you can restore them in Settings."
        );
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
            duplicate_scan_enabled: self.duplicate_scan_enabled,
            duplicate_check_exact: self.duplicate_check_exact,
            duplicate_check_recordings: self.duplicate_check_recordings,
            duplicate_check_partial: self.duplicate_check_partial,
            deleted_songs: self.deleted_songs.clone(),
            auto_purge_deleted: self.auto_purge_deleted,
            scan_rate_per_second: self.scan_rate_per_second,
            daily_mixes: self.daily_mix_paths.to_vec(),
            show_queue_on_start: !self.queue_collapsed,
            compact_mode: self.compact_mode,
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
        self.home_cache_dirty = true;
        self.save_state();
        // Like and dislike both affect the liked set (disliking removes a like),
        // so refresh duplicate suggestions on the cancellable worker thread.
        self.start_duplicate_scan();
    }

    fn toggle_artist_like(&mut self, artist: &str) {
        let key = artist.to_lowercase();
        if !self.liked_artists.remove(&key) {
            self.liked_artists.insert(key);
        }
        self.recommendations_dirty = true;
        self.home_cache_dirty = true;
        self.save_state();
    }

    fn toggle_album_like(&mut self, album: &str) {
        let key = album.to_lowercase();
        if !self.liked_albums.remove(&key) {
            self.liked_albums.insert(key);
        }
        self.recommendations_dirty = true;
        self.home_cache_dirty = true;
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
            Page::Discover => Vec::new(),
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
        self.home_cache_dirty = true;
        self.history.insert(0, track_path);
        self.history.truncate(500);
        self.recommendations_dirty = true;
        self.home_cache_dirty = true;
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
        let accent = accent_color(&self.accent);
        let is_playing = self
            .sink
            .as_ref()
            .is_some_and(|sink| !sink.is_paused() && !sink.empty());
        // A restrained breathing highlight on the playing row. It reuses the
        // normal playback repaint cadence, so it does not add another timer.
        let active_pulse = if active && is_playing {
            let phase = ui.ctx().input(|input| input.time) as f32;
            0.92 + (phase * 2.4).sin().abs() * 0.08
        } else {
            1.0
        };
        let base_fill = if active {
            Color32::from_rgb(24, 35, 78).linear_multiply(active_pulse)
        } else {
            Color32::TRANSPARENT
        };
        let fill = base_fill.linear_multiply(1.0 - hover_mix)
            + Color32::from_rgb(24, 24, 24).linear_multiply(hover_mix);
        let stroke = if active || hovered {
            egui::Stroke::new(
                1.0_f32,
                accent.linear_multiply(if active {
                    0.58 + active_pulse * 0.12
                } else {
                    0.25
                }),
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
                if active || hover_mix > 0.01 {
                    let stripe = egui::Rect::from_min_size(
                        row_rect.min + Vec2::new(0.0, 12.0),
                        Vec2::new(3.0, row_height - 24.0),
                    );
                    ui.painter().rect_filled(
                        stripe,
                        egui::CornerRadius::same(2),
                        accent.linear_multiply(if active { 0.85 } else { hover_mix * 0.45 }),
                    );
                }
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
        row_response
            .clone()
            .on_hover_text("Right-click for more song actions");
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
            self.home_cache_dirty = true;
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

    fn show_now_playing_view(&mut self, ui: &mut egui::Ui) {
        let current_position = self.current;
        let current_index = current_position
            .and_then(|position| self.queue.get(position))
            .copied();
        let current_track = current_index
            .and_then(|index| self.tracks.get(index))
            .cloned();
        let cover = current_index.and_then(|index| self.artwork_texture(ui.ctx(), index));
        let accent = accent_color(&self.accent);

        let mut suggestions = self.recommendations.clone();
        suggestions.retain(|&index| {
            Some(index) != current_index
                && self.tracks.get(index).is_some_and(|track| {
                    !self.disliked_songs.contains(&track.path)
                        && !self.excluded_tracks.contains(&track.path)
                })
        });
        if suggestions.len() < 12 {
            for (index, track) in self.tracks.iter().enumerate() {
                if Some(index) != current_index
                    && !suggestions.contains(&index)
                    && !self.disliked_songs.contains(&track.path)
                    && !self.excluded_tracks.contains(&track.path)
                {
                    suggestions.push(index);
                    if suggestions.len() >= 36 {
                        break;
                    }
                }
            }
        }
        if let Some(current) = &current_track {
            suggestions.sort_by_key(|&index| {
                self.tracks.get(index).map_or(2, |track| {
                    if !current.genre.is_empty() && track.genre.eq_ignore_ascii_case(&current.genre)
                    {
                        0
                    } else if !current.artist.is_empty()
                        && track.artist.eq_ignore_ascii_case(&current.artist)
                    {
                        1
                    } else {
                        2
                    }
                })
            });
        }

        let mut close_view = false;
        let mut play_queue_position = None;
        let mut play_recommendation = None;
        let mut add_recommendation = None;
        let mut play_radio = false;
        let mut add_radio = false;
        let mut like_current = false;
        let mut dislike_current = false;

        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new("NOW PLAYING").small().strong().color(accent));
                ui.heading("Your listening session");
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Close player").clicked() {
                    close_view = true;
                }
                if ui
                    .selectable_label(self.now_playing_tab == NowPlayingTab::ForYou, "For you")
                    .clicked()
                {
                    self.now_playing_tab = NowPlayingTab::ForYou;
                }
                if ui
                    .selectable_label(self.now_playing_tab == NowPlayingTab::UpNext, "Up next")
                    .clicked()
                {
                    self.now_playing_tab = NowPlayingTab::UpNext;
                }
            });
        });
        ui.add_space(10.0);

        ui.columns(2, |columns| {
            columns[0].vertical_centered(|ui| {
                let art_size = (ui.available_width().min(ui.available_height() - 130.0) * 0.82)
                    .clamp(170.0, 430.0);
                if let Some(cover) = &cover {
                    ui.add(
                        egui::Image::new(cover)
                            .fit_to_exact_size(Vec2::splat(art_size))
                            .corner_radius(egui::CornerRadius::same(12)),
                    );
                } else {
                    let (rect, _) =
                        ui.allocate_exact_size(Vec2::splat(art_size), egui::Sense::hover());
                    paint_cover_or_placeholder(ui, None, rect, current_index.unwrap_or(0));
                }
                ui.add_space(18.0);
                if let Some(track) = &current_track {
                    ui.label(
                        RichText::new(&track.title)
                            .size(24.0)
                            .strong()
                            .color(Color32::WHITE),
                    );
                    ui.label(
                        RichText::new(display_credit(&track.artist))
                            .size(16.0)
                            .color(Color32::from_rgb(184, 187, 198)),
                    );
                    if !track.album.is_empty() {
                        ui.label(RichText::new(&track.album).small().color(Color32::GRAY));
                    }
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add(
                                egui::Button::image(
                                    egui::Image::new(if self.liked_songs.contains(&track.path) {
                                        &self.ui_icons["like"]
                                    } else {
                                        &self.ui_icons["like_outline"]
                                    })
                                    .fit_to_exact_size(Vec2::splat(22.0)),
                                )
                                .frame(false),
                            )
                            .on_hover_text("Like song")
                            .clicked()
                        {
                            like_current = true;
                        }
                        if ui
                            .add(
                                egui::Button::image(
                                    egui::Image::new(&self.ui_icons["dislike"])
                                        .fit_to_exact_size(Vec2::splat(22.0)),
                                )
                                .selected(self.disliked_songs.contains(&track.path)),
                            )
                            .on_hover_text("Dislike song")
                            .clicked()
                        {
                            dislike_current = true;
                        }
                        if ui.button("Start radio").clicked() {
                            play_radio = true;
                        }
                        if ui.button("Add recommendations").clicked() {
                            add_radio = true;
                        }
                    });
                } else {
                    ui.label(RichText::new("Nothing is playing").size(22.0).strong());
                    ui.label("Start a song or mix and your player will appear here.");
                }
            });

            egui::Frame::new()
                .fill(Color32::from_rgb(17, 19, 26))
                .corner_radius(egui::CornerRadius::same(14))
                .inner_margin(egui::Margin::symmetric(16, 14))
                .show(&mut columns[1], |ui| match self.now_playing_tab {
                    NowPlayingTab::UpNext => {
                        let start = current_position.unwrap_or(0);
                        let queue_items = self
                            .queue
                            .iter()
                            .enumerate()
                            .skip(start)
                            .filter_map(|(position, &index)| {
                                self.tracks
                                    .get(index)
                                    .cloned()
                                    .map(|track| (position, index, track))
                            })
                            .collect::<Vec<_>>();
                        let queue_is_empty = queue_items.is_empty();
                        ui.horizontal(|ui| {
                            ui.heading("Up next");
                            ui.label(
                                RichText::new(format!("{} tracks", queue_items.len()))
                                    .small()
                                    .color(Color32::GRAY),
                            );
                        });
                        ui.label(
                            RichText::new("Your queue, followed by music picked for you.")
                                .small()
                                .color(Color32::GRAY),
                        );
                        ui.add_space(8.0);
                        egui::ScrollArea::vertical()
                            .id_salt("now-playing-up-next")
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                for (position, index, track) in queue_items {
                                    let selected = Some(position) == current_position;
                                    egui::Frame::new()
                                        .fill(if selected {
                                            accent.linear_multiply(0.18)
                                        } else {
                                            Color32::TRANSPARENT
                                        })
                                        .corner_radius(egui::CornerRadius::same(8))
                                        .inner_margin(egui::Margin::symmetric(8, 5))
                                        .show(ui, |ui| {
                                            ui.horizontal(|ui| {
                                                if let Some(texture) =
                                                    self.artwork_texture(ui.ctx(), index)
                                                {
                                                    ui.add(
                                                        egui::Image::new(&texture)
                                                            .fit_to_exact_size(Vec2::splat(42.0))
                                                            .corner_radius(
                                                                egui::CornerRadius::same(5),
                                                            ),
                                                    );
                                                } else {
                                                    let (rect, _) = ui.allocate_exact_size(
                                                        Vec2::splat(42.0),
                                                        egui::Sense::hover(),
                                                    );
                                                    paint_cover_or_placeholder(
                                                        ui, None, rect, index,
                                                    );
                                                }
                                                ui.vertical(|ui| {
                                                    if ui
                                                        .selectable_label(
                                                            selected,
                                                            RichText::new(&track.title).strong(),
                                                        )
                                                        .clicked()
                                                    {
                                                        play_queue_position = Some(position);
                                                    }
                                                    ui.label(
                                                        RichText::new(display_credit(
                                                            &track.artist,
                                                        ))
                                                        .small()
                                                        .color(Color32::GRAY),
                                                    );
                                                });
                                                ui.with_layout(
                                                    egui::Layout::right_to_left(
                                                        egui::Align::Center,
                                                    ),
                                                    |ui| {
                                                        ui.label(
                                                            RichText::new(format_time(
                                                                track.duration,
                                                            ))
                                                            .small()
                                                            .color(Color32::GRAY),
                                                        );
                                                    },
                                                );
                                            });
                                        });
                                }
                                if queue_is_empty {
                                    ui.add_space(18.0);
                                    ui.label("Nothing is queued after this song.");
                                    ui.label(
                                        RichText::new("Switch to For you to build a radio queue.")
                                            .small()
                                            .color(Color32::GRAY),
                                    );
                                }
                            });
                    }
                    NowPlayingTab::ForYou => {
                        ui.horizontal(|ui| {
                            ui.heading("Picked for you");
                            if ui.button("▶  Play radio").clicked() {
                                play_radio = true;
                            }
                            if ui.button("Add all").clicked() {
                                add_radio = true;
                            }
                        });
                        let context = current_track.as_ref().map_or_else(
                            || "Based on your liked music and listening history.".to_string(),
                            |track| {
                                if !track.genre.is_empty() {
                                    format!(
                                        "A mix inspired by {} and {}.",
                                        track.title, track.genre
                                    )
                                } else {
                                    format!("A mix inspired by {}.", track.title)
                                }
                            },
                        );
                        ui.label(RichText::new(context).small().color(Color32::GRAY));
                        ui.add_space(8.0);
                        egui::ScrollArea::vertical()
                            .id_salt("now-playing-for-you")
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                for &index in suggestions.iter().take(36) {
                                    let Some(track) = self.tracks.get(index).cloned() else {
                                        continue;
                                    };
                                    ui.horizontal(|ui| {
                                        if let Some(texture) = self.artwork_texture(ui.ctx(), index)
                                        {
                                            ui.add(
                                                egui::Image::new(&texture)
                                                    .fit_to_exact_size(Vec2::splat(42.0))
                                                    .corner_radius(egui::CornerRadius::same(5)),
                                            );
                                        } else {
                                            let (rect, _) = ui.allocate_exact_size(
                                                Vec2::splat(42.0),
                                                egui::Sense::hover(),
                                            );
                                            paint_cover_or_placeholder(ui, None, rect, index);
                                        }
                                        if ui
                                            .selectable_label(
                                                false,
                                                RichText::new(format!(
                                                    "{}\n{}",
                                                    track.title,
                                                    display_credit(&track.artist)
                                                )),
                                            )
                                            .clicked()
                                        {
                                            play_recommendation = Some(index);
                                        }
                                        if ui.small_button("Add").clicked() {
                                            add_recommendation = Some(index);
                                        }
                                    });
                                }
                                if suggestions.is_empty() {
                                    ui.label("Add music to your library to build recommendations.");
                                }
                            });
                    }
                });
        });

        if close_view {
            self.now_playing_expanded = false;
        }
        if let Some(position) = play_queue_position {
            self.play_queue_at(position);
        }
        if let Some(index) = play_recommendation {
            self.queue_track_next(index);
            let position = self.current.map_or(0, |current| current + 1);
            if !self.queue.is_empty() {
                self.play_queue_at(position.min(self.queue.len() - 1));
            }
        }
        if let Some(index) = add_recommendation {
            self.queue_track_next(index);
        }
        if add_radio {
            self.add_tracks_to_queue(&suggestions);
        }
        if play_radio && !suggestions.is_empty() {
            self.play_indices(suggestions, 0);
            self.now_playing_tab = NowPlayingTab::UpNext;
        }
        if let Some(index) = current_index {
            if like_current {
                self.toggle_song_reaction(index, true);
            }
            if dislike_current {
                self.toggle_song_reaction(index, false);
            }
        }
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
            Page::Discover => "Discover".to_string(),
            Page::Songs => "Quick picks".to_string(),
            Page::Albums => "Albums".to_string(),
            Page::Artists => "Artists".to_string(),
            Page::ArtistDetail => self.selected_artist.clone().unwrap_or("Artist".into()),
            Page::AlbumDetail => self.selected_album.clone().unwrap_or("Album".into()),
            Page::LikedSongs => "Liked music".to_string(),
            Page::LikedArtists => "Liked artists".to_string(),
            Page::LikedAlbums => "Saved albums".to_string(),
            Page::Playlists => self
                .selected_playlist
                .and_then(|index| self.playlists.get(index))
                .map(|playlist| playlist.name.clone())
                .unwrap_or_else(|| "Playlists".to_string()),
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
                        self.home_cache_dirty = true;
                        self.home_cache_dirty = true;
                        self.save_state();
                    }
                    if self.page != Page::Settings
                        && self.page != Page::Playlists
                        && self.page != Page::Discover
                    {
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
                    egui::ScrollArea::vertical()
                        .id_salt("lyrics-content")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
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
            Page::Discover => self.show_discover(ui),
            Page::Songs | Page::LikedSongs | Page::History | Page::Disliked => {
                if self.page == Page::Songs {
                    self.show_quick_picks(ui, &indices);
                } else {
                    self.show_track_collection(ui, &indices, "tracks");
                }
            }
            Page::Playlists if self.selected_playlist.is_none() => self.show_playlist_library(ui),
            Page::Playlists => self.show_playlist_detail(ui, &indices),
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

    fn mix_context(&self) -> MixContext {
        MixContext {
            tracks: self.tracks.clone(),
            history: self.history.clone(),
            liked_songs: self.liked_songs.clone(),
            disliked_songs: self.disliked_songs.clone(),
            excluded_tracks: self.excluded_tracks.clone(),
            liked_artists: self.liked_artists.clone(),
            liked_albums: self.liked_albums.clone(),
            discovery: self.discovery,
            prefer_unplayed: self.prefer_unplayed,
            recommendation_variety: self.recommendation_variety,
            recommendation_size: self.recommendation_size,
            home_genre: self.home_genre.clone(),
        }
    }

    #[cfg(test)]
    fn build_daily_mixes(&self, seed: u64) -> [Vec<usize>; 3] {
        self.mix_context().build_daily_mixes(seed)
    }

    fn next_mix_seed(&mut self) -> u64 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let today = now.as_secs() / 86_400;
        let mut seed = (now.as_nanos() as u64)
            ^ self.daily_mix_seed.rotate_left(17)
            ^ today.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        if seed == 0 {
            seed = 0xA076_1D64_78BD_642F;
        }
        seed
    }

    // Home-only background work. Recommendations are (re)computed when marked
    // dirty; mixes are generated ONLY when the user pressed Refresh, or once
    // when no mixes have ever been saved. Nothing here runs on the UI thread
    // except taking the snapshot.
    fn drive_home_jobs(&mut self, ctx: &egui::Context) {
        if !ctx.input(|input| input.focused) {
            return;
        }
        if self.home_job_receiver.is_some() {
            return;
        }
        if self.tracks.is_empty() {
            self.mix_refresh_requested = false;
            return;
        }
        let first_run_mixes =
            !self.initial_mixes_attempted && self.daily_mix_paths.iter().all(Vec::is_empty);
        let want_mixes = self.mix_refresh_requested || first_run_mixes;
        let want_recommendations = self.recommendations_dirty;
        if !want_mixes && !want_recommendations {
            return;
        }
        if want_mixes {
            self.mix_refresh_requested = false;
            self.initial_mixes_attempted = true;
        }
        let seed = self.next_mix_seed();
        let revision = self.home_data_revision;
        let context = self.mix_context();
        let repaint = ctx.clone();
        let (sender, receiver) = mpsc::channel();
        self.home_job_receiver = Some(receiver);
        self.mix_refresh_running = want_mixes;
        thread::spawn(move || {
            let recommendations = if want_recommendations {
                Some(context.compute_recommendations())
            } else {
                None
            };
            let mixes = if want_mixes {
                let built = context.build_daily_mixes(seed);
                Some(built.map(|mix| {
                    mix.into_iter()
                        .filter_map(|index| {
                            context.tracks.get(index).map(|track| track.path.clone())
                        })
                        .collect::<Vec<_>>()
                }))
            } else {
                None
            };
            let _ = sender.send(HomeJobResult {
                revision,
                recommendations,
                mixes,
                seed,
            });
            request_repaint_when_focused(&repaint);
        });
    }

    fn poll_home_job(&mut self) {
        let Some(receiver) = &self.home_job_receiver else {
            return;
        };
        match receiver.try_recv() {
            Ok(result) => {
                self.home_job_receiver = None;
                self.mix_refresh_running = false;
                // A library rescan while the job ran invalidates track indices.
                if result.revision == self.home_data_revision {
                    if let Some(recommendations) = result.recommendations {
                        self.recommendations = recommendations;
                        self.recommendations_dirty = false;
                    }
                }
                if let Some(mixes) = result.mixes {
                    self.daily_mix_paths = mixes;
                    self.daily_mix_seed = result.seed;
                    self.save_state();
                }
                self.home_cache_dirty = true;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.home_job_receiver = None;
                self.mix_refresh_running = false;
            }
        }
    }

    fn resolve_daily_mixes(&mut self) {
        let by_path = self
            .tracks
            .iter()
            .enumerate()
            .map(|(index, track)| (track.path.as_path(), index))
            .collect::<HashMap<_, _>>();
        let resolved: [Vec<usize>; 3] = std::array::from_fn(|slot| {
            self.daily_mix_paths[slot]
                .iter()
                .filter_map(|path| by_path.get(path.as_path()).copied())
                .filter(|&index| {
                    let path = &self.tracks[index].path;
                    !self.excluded_tracks.contains(path) && !self.disliked_songs.contains(path)
                })
                .collect()
        });
        self.daily_mixes = resolved;
    }

    fn open_song_properties(&mut self, index: usize, ctx: &egui::Context) {
        let Some(track) = self.tracks.get(index) else {
            return;
        };
        let path = track.path.clone();
        self.properties_view = Some(SongPropertiesView {
            path: path.clone(),
            title: track.title.clone(),
            artist: track.artist.clone(),
            album: track.album.clone(),
            genre: track.genre.clone(),
            year: track.year,
            duration: track.duration,
            details: None,
        });
        // File size, format, bitrate and sample rate are only read when the
        // window is opened, and off the UI thread.
        let (sender, receiver) = mpsc::channel();
        self.properties_receiver = Some(receiver);
        let repaint = ctx.clone();
        thread::spawn(move || {
            let details = read_file_details(&path);
            let _ = sender.send((path, details));
            request_repaint_when_focused(&repaint);
        });
    }

    fn show_properties_window(&mut self, ctx: &egui::Context) {
        if self.properties_view.is_none() {
            return;
        }
        if let Some(receiver) = &self.properties_receiver {
            match receiver.try_recv() {
                Ok((path, details)) => {
                    self.properties_receiver = None;
                    if let Some(view) = &mut self.properties_view {
                        if view.path == path {
                            view.details = Some(details);
                        }
                    }
                }
                Err(TryRecvError::Empty) => {
                    if ctx.input(|input| input.focused) {
                        ctx.request_repaint_after(Duration::from_millis(50));
                    }
                }
                Err(TryRecvError::Disconnected) => {
                    self.properties_receiver = None;
                    if let Some(view) = &mut self.properties_view {
                        if view.details.is_none() {
                            view.details = Some(FileDetails {
                                error: Some("Could not load file details.".into()),
                                ..FileDetails::default()
                            });
                        }
                    }
                }
            }
        }
        let mut open = true;
        let mut close_clicked = false;
        if let Some(view) = &self.properties_view {
            egui::Window::new("Song properties")
                .id(egui::Id::new("song-properties-window"))
                .open(&mut open)
                .collapsible(false)
                .resizable(true)
                .default_width(430.0)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    egui::Grid::new("song-properties-grid")
                        .num_columns(2)
                        .spacing([18.0, 8.0])
                        .striped(true)
                        .show(ui, |ui| {
                            properties_row(ui, "Title", &display_metadata(&view.title));
                            properties_row(ui, "Artist", &display_credit(&view.artist));
                            properties_row(ui, "Album", &display_metadata(&view.album));
                            properties_row(ui, "Genre", &display_metadata(&view.genre));
                            properties_row(
                                ui,
                                "Year",
                                &if view.year > 0 {
                                    view.year.to_string()
                                } else {
                                    "—".into()
                                },
                            );
                            properties_row(
                                ui,
                                "Duration",
                                &if view.duration > 0.0 {
                                    format_time(view.duration)
                                } else {
                                    "—".into()
                                },
                            );
                            properties_row(ui, "File path", &view.path.display().to_string());
                            match &view.details {
                                None => {
                                    properties_row(ui, "File details", "Loading…");
                                }
                                Some(details) => {
                                    properties_row(
                                        ui,
                                        "File size",
                                        &details.size_bytes.map_or("—".into(), |bytes| {
                                            format!("{} ({bytes} bytes)", format_file_size(bytes))
                                        }),
                                    );
                                    properties_row(
                                        ui,
                                        "File format",
                                        details.format.as_deref().unwrap_or("—"),
                                    );
                                    properties_row(
                                        ui,
                                        "Bitrate",
                                        &details
                                            .bitrate_kbps
                                            .map_or("—".into(), |kbps| format!("{kbps} kbps")),
                                    );
                                    properties_row(
                                        ui,
                                        "Sample rate",
                                        &details
                                            .sample_rate_hz
                                            .map_or("—".into(), |hz| format!("{hz} Hz")),
                                    );
                                    properties_row(
                                        ui,
                                        "Channels",
                                        &details
                                            .channels
                                            .map_or("—".into(), |channels| channels.to_string()),
                                    );
                                    properties_row(
                                        ui,
                                        "Bit depth",
                                        &details
                                            .bit_depth
                                            .map_or("—".into(), |bits| format!("{bits}-bit")),
                                    );
                                    properties_row(
                                        ui,
                                        "Last modified",
                                        &details.modified.map_or("—".into(), format_elapsed_since),
                                    );
                                }
                            }
                        });
                    if let Some(error) = view.details.as_ref().and_then(|d| d.error.as_ref()) {
                        ui.add_space(6.0);
                        ui.colored_label(Color32::LIGHT_RED, error);
                    }
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.button("Copy file path").clicked() {
                            ui.ctx().copy_text(view.path.to_string_lossy().into_owned());
                        }
                        if ui.button("Close").clicked() {
                            close_clicked = true;
                        }
                    });
                });
        }
        if !open || close_clicked {
            self.properties_view = None;
            self.properties_receiver = None;
        }
    }

    fn build_home_cache(&mut self) -> HomeViewCache {
        self.resolve_daily_mixes();
        // Recommendations are computed by the Home background job; never
        // compute them synchronously while drawing Home.
        let recommendations = self
            .recommendations
            .iter()
            .copied()
            .filter(|&index| index < self.tracks.len())
            .collect::<Vec<_>>();
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
        let welcome_resume = if self.history.is_empty() {
            None
        } else {
            recent.first().and_then(|&index| {
                self.tracks
                    .get(index)
                    .map(|track| (index, track.title.clone()))
            })
        };
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
        let mix_subtitles = [
            "Your core rotation · favorites first",
            "Familiar sounds · fresh discoveries",
            "Deep cuts · left-field picks",
        ];
        let daily_mix_cards = self
            .daily_mixes
            .iter()
            .enumerate()
            .filter_map(|(position, tracks)| {
                let &cover_index = tracks.first()?;
                Some(HomeCard {
                    title: format!("My Mix {}", position + 1),
                    subtitle: mix_subtitles[position].into(),
                    cover_index,
                    tracks: tracks.clone(),
                    start: 0,
                })
            })
            .collect::<Vec<_>>();
        let mut mixes = Vec::new();
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
        genres.truncate(24);
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
            .take(16)
            .collect::<Vec<_>>();
        let mut decade_tracks = BTreeMap::<u32, Vec<usize>>::new();
        for &index in &available {
            let year = self.tracks[index].year;
            if year >= 1950 {
                decade_tracks
                    .entry((year / 10) * 10)
                    .or_default()
                    .push(index);
            }
        }
        let decade_mix_cards = decade_tracks
            .into_iter()
            .rev()
            .filter_map(|(decade, mut tracks)| {
                if tracks.len() < 4 {
                    return None;
                }
                tracks.sort_by_key(|index| {
                    recommendation_order
                        .get(index)
                        .copied()
                        .unwrap_or(usize::MAX)
                });
                tracks.truncate(self.recommendation_size);
                let &cover_index = tracks.first()?;
                Some(HomeCard {
                    title: format!("{decade}s Mix"),
                    subtitle: format!("{} tracks from the {decade}s", tracks.len()),
                    cover_index,
                    tracks,
                    start: 0,
                })
            })
            .take(12)
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
        HomeViewCache {
            genres,
            daily_mix_cards,
            recent_cards,
            mixes,
            unplayed_cards,
            genre_mix_cards,
            decade_mix_cards,
            library_cards,
            latest_cards,
            welcome_resume,
        }
    }

    fn show_discover(&mut self, ui: &mut egui::Ui) {
        self.drive_home_jobs(ui.ctx());
        if self.home_job_receiver.is_some() && ui.ctx().input(|input| input.focused) {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
        if self.home_cache_dirty || self.home_cache.is_none() {
            self.home_cache = Some(self.build_home_cache());
            self.home_cache_dirty = false;
        }
        let home = self.home_cache.take().unwrap_or_default();
        let mut refresh = false;

        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.heading(RichText::new("Find your next favorite").size(24.0).strong());
                ui.label(
                    RichText::new(
                        "Personalized radio, genres, eras, and fresh picks from your library.",
                    )
                    .small()
                    .color(Color32::GRAY),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                refresh = ui
                    .add_enabled(
                        !self.tracks.is_empty() && self.home_job_receiver.is_none(),
                        egui::Button::new("↻ Refresh discovery"),
                    )
                    .clicked();
            });
        });
        ui.add_space(8.0);
        if self.tracks.is_empty() {
            ui.label("Add a music folder to build personalized mixes.");
        }
        egui::ScrollArea::vertical()
            .id_salt("discover-feed")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                self.show_home_shelf(
                    ui,
                    "Daily mixes",
                    "THREE PERSONALIZED ROTATIONS",
                    &home.daily_mix_cards,
                    None,
                );
                ui.add_space(18.0);
                self.show_home_shelf(
                    ui,
                    "Mixes from your genres",
                    "BUILT FROM THE TAGS IN YOUR LIBRARY",
                    &home.genre_mix_cards,
                    None,
                );
                ui.add_space(18.0);
                self.show_home_shelf(
                    ui,
                    "By decade",
                    "REVISIT MUSIC FROM DIFFERENT ERAS",
                    &home.decade_mix_cards,
                    None,
                );
                ui.add_space(18.0);
                self.show_home_shelf(
                    ui,
                    "Artist radio and favorites",
                    "MORE FROM ARTISTS AND SONGS YOU LIKE",
                    &home.mixes,
                    None,
                );
                ui.add_space(18.0);
                self.show_home_shelf(
                    ui,
                    "New to you",
                    "RECOMMENDED TRACKS YOU HAVEN’T PLAYED YET",
                    &home.unplayed_cards,
                    None,
                );
                ui.add_space(18.0);
                self.show_home_shelf(
                    ui,
                    "Listen again",
                    "YOUR RECENT ROTATION",
                    &home.recent_cards,
                    Some(Page::History),
                );
            });
        self.home_cache = Some(home);
        if refresh {
            self.recommendations_dirty = true;
            self.home_cache_dirty = true;
            self.mix_refresh_requested = true;
            self.drive_home_jobs(ui.ctx());
        }
    }

    fn show_home(&mut self, ui: &mut egui::Ui) {
        self.drive_home_jobs(ui.ctx());
        if (self.home_job_receiver.is_some() || self.mix_refresh_requested)
            && ui.ctx().input(|input| input.focused)
        {
            ui.ctx().request_repaint_after(Duration::from_millis(80));
        }
        let mix_busy = self.mix_refresh_requested || self.mix_refresh_running;
        let mut refresh_mixes = false;
        let hero_intro =
            ui.ctx()
                .animate_bool_with_time(egui::Id::new("home-hero-intro"), true, 0.42);
        let hero_glow = 0.10 + hero_intro * 0.12;
        egui::Frame::NONE
            .fill(mix_color(
                Color32::from_rgb(18, 19, 34),
                Color32::from_rgb(34, 31, 67),
                hero_glow,
            ))
            .stroke(egui::Stroke::new(
                1.0_f32,
                accent_color(&self.accent).linear_multiply(0.18 + hero_intro * 0.22),
            ))
            .corner_radius(egui::CornerRadius::same(18))
            .inner_margin(egui::Margin::symmetric(18, 12))
            .show(ui, |ui| {
                ui.set_opacity(hero_intro);
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("YOUR LISTENING, REIMAGINED")
                                .small()
                                .strong()
                                .color(Color32::from_rgb(164, 167, 224)),
                        );
                        ui.label(
                            RichText::new("A soundtrack that moves with you")
                                .size(23.0)
                                .strong(),
                        );
                        ui.label(
                            RichText::new(format!(
                                "{} songs in your library · 3 evolving daily mixes",
                                self.tracks.len()
                            ))
                            .small()
                            .color(Color32::from_rgb(180, 180, 194)),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let accent_fill = accent_color(&self.accent);
                        refresh_mixes = ui
                            .add_enabled_ui(!mix_busy, |ui| {
                                ui.add_sized(
                                    [146.0, 36.0],
                                    egui::Button::new(if mix_busy {
                                        "Refreshing mixes…"
                                    } else {
                                        "↻  Refresh mixes"
                                    })
                                    .fill(accent_fill),
                                )
                            })
                            .inner
                            .clicked();
                        if mix_busy {
                            ui.add(egui::Spinner::new());
                        }
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new("FRESH PICKS")
                                .small()
                                .strong()
                                .color(Color32::from_rgb(173, 174, 205)),
                        );
                    });
                });
            });
        if refresh_mixes {
            self.mix_refresh_requested = true;
            self.drive_home_jobs(ui.ctx());
        }
        ui.add_space(12.0);
        if self.recommendations.is_empty()
            && !self.tracks.is_empty()
            && (self.recommendations_dirty || self.home_job_receiver.is_some())
        {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new());
                ui.label(RichText::new("Preparing your Home…").color(Color32::GRAY));
            });
            return;
        }
        if self.home_cache_dirty || self.home_cache.is_none() {
            let rebuilt = self.build_home_cache();
            self.home_cache = Some(rebuilt);
            self.home_cache_dirty = false;
        }
        // Take the cache temporarily so shelf rendering can mutably access the
        // player (artwork cache, playback, context menus) without cloning cards.
        let home = self.home_cache.take().unwrap_or_default();
        let welcome_summary = format!(
            "{} songs in your library · {} liked",
            self.tracks.len(),
            self.liked_songs.len()
        );

        egui::ScrollArea::vertical()
            .id_salt("home-feed-v2")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let page_rect = ui.max_rect();
                let band_height = 72.0;
                let gradient = [
                    Color32::from_rgb(22, 23, 48),
                    Color32::from_rgb(18, 17, 38),
                    Color32::from_rgb(12, 13, 25),
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
                let mut resume_play = None;
                egui::Frame::NONE
                    .fill(Color32::from_rgb(24, 25, 44))
                    .stroke(egui::Stroke::new(
                        1.0_f32,
                        accent_color(&self.accent).linear_multiply(0.25),
                    ))
                    .corner_radius(egui::CornerRadius::same(14))
                    .inner_margin(egui::Margin::symmetric(18, 12))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.label(
                                    RichText::new(if home.welcome_resume.is_some() {
                                        "Welcome back"
                                    } else {
                                        "Welcome to Lilt"
                                    })
                                    .size(22.0)
                                    .strong(),
                                );
                                ui.label(
                                    RichText::new(&welcome_summary)
                                        .small()
                                        .color(Color32::from_rgb(180, 180, 194)),
                                );
                            });
                            if let Some((index, title)) = &home.welcome_resume {
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui
                                            .add(egui::Button::new(format!("▶  Continue: {title}")))
                                            .clicked()
                                        {
                                            resume_play = Some(*index);
                                        }
                                    },
                                );
                            }
                        });
                    });
                if let Some(index) = resume_play {
                    self.play_indices(vec![index], 0);
                }
                ui.add_space(12.0);
                egui::ScrollArea::horizontal()
                    .id_salt("home-genre-chips")
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let mut chips = vec!["All music".to_string()];
                            chips.extend(home.genres.iter().map(|(genre, _)| genre.clone()));
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
                                            Color32::from_rgb(51, 62, 116)
                                        } else {
                                            Color32::from_rgb(32, 34, 55)
                                        }),
                                    )
                                    .clicked()
                                {
                                    self.home_genre = if genre == "All music" {
                                        String::new()
                                    } else {
                                        genre
                                    };
                                    self.home_cache_dirty = true;
                                }
                            }
                        });
                    });
                ui.add_space(14.0);
                self.show_home_shelf(
                    ui,
                    "Daily Mixes",
                    "THREE PERSONALIZED ROTATIONS · UPDATED DAILY OR ON REFRESH",
                    &home.daily_mix_cards,
                    None,
                );
                ui.add_space(16.0);
                self.show_home_shelf(
                    ui,
                    "Listen again",
                    "YOUR RECENT ROTATION",
                    &home.recent_cards,
                    Some(Page::History),
                );
                ui.add_space(16.0);
                self.show_home_shelf(
                    ui,
                    "Made for your library",
                    "ARTIST RADIO, FAVORITES, AND FRESH ADDITIONS",
                    &home.mixes,
                    None,
                );
                ui.add_space(16.0);
                self.show_home_shelf(
                    ui,
                    "New to you",
                    "RECOMMENDED SONGS YOU HAVEN'T PLAYED YET",
                    &home.unplayed_cards,
                    None,
                );
                ui.add_space(16.0);
                self.show_home_shelf(
                    ui,
                    "Your genre mixes",
                    "PICK A SOUND FROM YOUR LIBRARY",
                    &home.genre_mix_cards,
                    None,
                );
                ui.add_space(16.0);
                self.show_home_shelf(
                    ui,
                    "From your library",
                    "ALBUMS READY FOR ANOTHER LISTEN",
                    &home.library_cards,
                    Some(Page::Albums),
                );
                ui.add_space(16.0);
                self.show_home_shelf(
                    ui,
                    "Recently added",
                    "NEW IN YOUR COLLECTION",
                    &home.latest_cards,
                    Some(Page::Songs),
                );
            });
        self.home_cache = Some(home);
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
                ui.ctx(),
                output.state.offset.y,
                output.inner_rect.height(),
                album_ids.len(),
                loaded,
                BATCH_SIZE,
                output.content_size.y,
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
            ui.ctx(),
            output.state.offset.y,
            output.inner_rect.height(),
            album_ids.len(),
            loaded,
            BATCH_SIZE,
            output.content_size.y,
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
            ui.ctx(),
            output.state.offset.y,
            output.inner_rect.height(),
            artists.len(),
            loaded,
            BATCH_SIZE,
            output.content_size.y,
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
        cards: &[HomeCard],
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
        // Only request a programmatic offset when an arrow is clicked. Feeding
        // the previous frame's offset back into ScrollArea every frame can
        // fight its own wheel/trackpad velocity and make scrolling feel sticky.
        let mut requested_shelf_scroll: Option<f32> = None;
        let mut selected_page = None;
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(
                    RichText::new(eyebrow)
                        .small()
                        .strong()
                        .color(Color32::from_rgb(170, 174, 212)),
                );
                ui.heading(RichText::new(title).size(26.0).strong());
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(page) = more_page {
                    if ui.button("More").clicked() {
                        selected_page = Some(page);
                    }
                }
                if ui.button("›").clicked() {
                    shelf_scroll += 520.0;
                    requested_shelf_scroll = Some(shelf_scroll);
                }
                if ui.button("‹").clicked() {
                    shelf_scroll = (shelf_scroll - 520.0).max(0.0);
                    requested_shelf_scroll = Some(shelf_scroll);
                }
            });
        });
        if let Some(page) = selected_page {
            self.page = page;
            self.selected_playlist = None;
        }
        ui.add_space(14.0);
        // A normal vertical wheel gesture should keep moving the Home feed,
        // even while the pointer is over a horizontal shelf. Hold Shift to
        // scroll the shelf itself; the ‹ / › controls always remain available.
        let shift_scroll = ui.input(|input| input.modifiers.shift);
        let mut shelf_area = egui::ScrollArea::horizontal()
            .id_salt(shelf_id)
            .auto_shrink([false, true])
            .enable_scrolling(shift_scroll);
        if let Some(offset) = requested_shelf_scroll {
            shelf_area = shelf_area.scroll_offset(Vec2::new(offset, 0.0));
        }
        let output = shelf_area.show(ui, |ui| {
            ui.horizontal(|ui| {
                for card in cards {
                    let is_daily_mix = card.title.starts_with("My Mix ");
                    let mut play = false;
                    let mut save_as_playlist = false;
                    ui.allocate_ui_with_layout(
                        Vec2::new(168.0, if is_daily_mix { 258.0 } else { 218.0 }),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            // Horizontal shelves can contain many cards. Skip all
                            // artwork requests, animations and widgets for cards
                            // outside the visible clip rectangle.
                            let card_rect = ui.max_rect();
                            if !ui.is_rect_visible(card_rect) {
                                // Keep the shelf's measured content size stable
                                // while skipping off-screen card work. Without
                                // reserving this rect, egui can shrink the feed
                                // during a frame and clamp the vertical scroll
                                // offset, making Home jump to another section.
                                ui.expand_to_include_rect(card_rect);
                                return;
                            }
                            let cover = self.artwork_texture(ui.ctx(), card.cover_index);
                            let entrance = ui.ctx().animate_bool_with_time(
                                egui::Id::new(("home-card-enter", title, &card.title)),
                                true,
                                0.34,
                            );
                            ui.set_opacity(entrance);
                            let (cover_rect, response) =
                                ui.allocate_exact_size(Vec2::splat(148.0), egui::Sense::click());
                            let hover_t = ui.ctx().animate_bool_with_time(
                                egui::Id::new(("home-card-hover", title, &card.title)),
                                response.hovered(),
                                0.18,
                            );
                            let animated_cover = cover_rect.expand(hover_t * 3.0);
                            let painter = ui.painter();
                            if hover_t > 0.01 {
                                painter.rect_filled(
                                    animated_cover.expand(3.0),
                                    egui::CornerRadius::same(13),
                                    accent_color(&self.accent).linear_multiply(hover_t * 0.20),
                                );
                            }
                            if let Some(cover) = &cover {
                                painter.image(
                                    cover.id(),
                                    animated_cover,
                                    egui::Rect::from_min_max(
                                        egui::Pos2::ZERO,
                                        egui::pos2(1.0, 1.0),
                                    ),
                                    Color32::WHITE,
                                );
                            } else {
                                let palette = [
                                    Color32::from_rgb(48, 66, 132),
                                    Color32::from_rgb(76, 54, 132),
                                    Color32::from_rgb(35, 105, 112),
                                    Color32::from_rgb(113, 79, 139),
                                    Color32::from_rgb(111, 91, 54),
                                ];
                                painter.rect_filled(
                                    animated_cover,
                                    egui::CornerRadius::same(10),
                                    palette[card.cover_index % palette.len()],
                                );
                                painter.text(
                                    animated_cover.center(),
                                    egui::Align2::CENTER_CENTER,
                                    "♫",
                                    egui::FontId::proportional(56.0),
                                    Color32::from_rgb(235, 230, 235),
                                );
                            }
                            if response.hovered() {
                                painter.rect_filled(
                                    animated_cover,
                                    egui::CornerRadius::same(10),
                                    Color32::from_black_alpha(58),
                                );
                            }
                            let center = animated_cover.center();
                            painter.circle_filled(
                                center,
                                25.0 + hover_t * 2.0,
                                Color32::from_black_alpha(150),
                            );
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
                            response.on_hover_text(format!(
                                "Play {} · right-click for more actions",
                                card.title
                            ));
                            ui.add_space(8.0);
                            ui.add_sized(
                                [162.0, 22.0],
                                egui::Label::new(RichText::new(&card.title).strong().size(15.0))
                                    .truncate(),
                            );
                            ui.add_sized(
                                [162.0, 32.0],
                                egui::Label::new(
                                    RichText::new(&card.subtitle)
                                        .size(12.0)
                                        .color(Color32::from_rgb(177, 174, 181)),
                                )
                                .truncate(),
                            );
                            if is_daily_mix {
                                save_as_playlist = ui
                                    .add_sized(
                                        [152.0, 30.0],
                                        egui::Button::new("＋  Save as playlist"),
                                    )
                                    .clicked();
                            }
                        },
                    );
                    if play {
                        self.play_indices(card.tracks.clone(), card.start);
                    }
                    if save_as_playlist {
                        self.create_playlist_from_tracks(&card.title, &card.tracks);
                    }
                }
            });
        });
        // Never persist a rubber-band/out-of-range shelf position. Keeping the
        // saved offset inside the real content bounds prevents the next arrow
        // press from starting from an invalid edge position.
        let max_shelf_scroll = (output.content_size.x - output.inner_rect.width()).max(0.0);
        self.home_shelf_offsets.insert(
            title.to_string(),
            output.state.offset.x.clamp(0.0, max_shelf_scroll),
        );
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
                ui.ctx(),
                output.state.offset.y,
                output.inner_rect.height(),
                indices.len(),
                loaded,
                BATCH_SIZE,
                output.content_size.y,
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
            ui.ctx(),
            output.state.offset.y,
            output.inner_rect.height(),
            indices.len(),
            loaded,
            BATCH_SIZE,
            output.content_size.y,
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
            ui.ctx(),
            output.state.offset.y,
            output.inner_rect.height(),
            indices.len(),
            loaded,
            BATCH_SIZE,
            output.content_size.y,
            232.0,
        );
    }

    fn show_playlist_detail(&mut self, ui: &mut egui::Ui, tracks: &[usize]) {
        let Some(total_count) = self
            .selected_playlist
            .and_then(|index| self.playlists.get(index))
            .map(|playlist| playlist.tracks.len())
        else {
            self.selected_playlist = None;
            self.show_playlist_library(ui);
            return;
        };
        let mut play = false;
        let mut shuffle = false;
        let mut enqueue = false;
        let mut add_now_playing = false;
        ui.label(
            RichText::new(format!(
                "{total_count} saved tracks · {} available in your library",
                tracks.len()
            ))
            .color(Color32::GRAY),
        );
        ui.horizontal(|ui| {
            if ui.button("← All playlists").clicked() {
                self.selected_playlist = None;
            }
            ui.separator();
            if ui
                .add_enabled(
                    !tracks.is_empty(),
                    egui::Button::new("▶  Play playlist").fill(accent_color(&self.accent)),
                )
                .clicked()
            {
                play = true;
            }
            if ui
                .add_enabled(!tracks.is_empty(), egui::Button::new("🔀  Shuffle"))
                .clicked()
            {
                shuffle = true;
            }
            if ui
                .add_enabled(!tracks.is_empty(), egui::Button::new("♫  Add to queue"))
                .clicked()
            {
                enqueue = true;
            }
            if ui.button("＋  Add now playing").clicked() {
                add_now_playing = true;
            }
        });
        if tracks.is_empty() {
            ui.label("This playlist has no available tracks. Add songs from the library or remove missing entries from its menu.");
        }
        if play {
            self.play_indices(tracks.to_vec(), 0);
        } else if shuffle {
            self.play_shuffled_indices(tracks.to_vec());
        } else if enqueue {
            self.add_tracks_to_queue(tracks);
        }
        if add_now_playing {
            let path = self
                .current
                .and_then(|position| self.queue.get(position))
                .and_then(|&track_index| self.tracks.get(track_index))
                .map(|track| track.path.clone());
            if let (Some(path), Some(playlist)) = (
                path,
                self.selected_playlist
                    .and_then(|index| self.playlists.get_mut(index)),
            ) {
                if !playlist.tracks.contains(&path) {
                    playlist.tracks.push(path);
                    self.save_state();
                }
            }
        }
        ui.separator();
        self.show_track_collection(ui, tracks, "playlist-detail");
    }

    fn show_playlist_library(&mut self, ui: &mut egui::Ui) {
        ui.label(
            RichText::new(
                "Select a playlist to open it. Use Play or Shuffle to start listening right away.",
            )
            .color(Color32::GRAY),
        );
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
        let mut play_next = false;
        let mut delete = None;
        let mut rename = None;
        let mut duplicate = None;
        let mut export = None;
        let mut copy_track_list = None;
        let mut remove_missing = None;
        let mut add_playing_to_playlist = None;
        let mut empty_playlist = None;
        let mut remove_playlist_tracks_from_library = None;
        let mut delete_playlist_files = None;
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
                    if ui.small_button("🔀 Shuffle").clicked() {
                        open = Some(index);
                        shuffle_play = true;
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
                if ui.button("⏭  Play playlist next").clicked() {
                    open = Some(index);
                    play_next = true;
                    ui.close_menu();
                }
                if ui.button("▤  Copy track list").clicked() {
                    copy_track_list = Some(index);
                    ui.close_menu();
                }
                if ui.button("⇩  Export playlist as M3U8…").clicked() {
                    export = Some(index);
                    ui.close_menu();
                }
                if ui.button("🧹  Remove missing tracks").clicked() {
                    remove_missing = Some(index);
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
                if ui.button("＋  Add currently playing song").clicked() {
                    add_playing_to_playlist = Some(index);
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("Empty playlist (keep library files)").clicked() {
                    empty_playlist = Some(index);
                    ui.close_menu();
                }
                if ui
                    .button("Remove all playlist songs from library (keep files)")
                    .clicked()
                {
                    remove_playlist_tracks_from_library = Some(index);
                    ui.close_menu();
                }
                if ui
                    .button(
                        RichText::new("Delete all playlist audio files permanently…")
                            .color(Color32::LIGHT_RED),
                    )
                    .clicked()
                {
                    delete_playlist_files = Some(index);
                    ui.close_menu();
                }
                ui.separator();
                if ui
                    .button(RichText::new("Delete playlist").color(Color32::LIGHT_RED))
                    .clicked()
                {
                    delete = Some(index);
                    ui.close_menu();
                }
            });
        }
        if let Some(index) = add_playing_to_playlist {
            let path = self
                .current
                .and_then(|position| self.queue.get(position))
                .and_then(|&track_index| self.tracks.get(track_index))
                .map(|track| track.path.clone());
            if let (Some(path), Some(playlist)) = (path, self.playlists.get_mut(index)) {
                if !playlist.tracks.contains(&path) {
                    playlist.tracks.push(path);
                    let name = playlist.name.clone();
                    self.save_state();
                    self.status = format!("Added the currently playing song to {name}.");
                }
            }
        }
        if let Some(index) = empty_playlist {
            if let Some(playlist) = self.playlists.get_mut(index) {
                let removed = playlist.tracks.len();
                playlist.tracks.clear();
                let name = playlist.name.clone();
                self.save_state();
                self.status = format!(
                    "Emptied {name}; removed {removed} playlist entries. Audio files were kept."
                );
            }
        }
        if let Some(index) = remove_playlist_tracks_from_library {
            if let Some(playlist) = self.playlists.get(index) {
                let paths = playlist.tracks.clone();
                let name = playlist.name.clone();
                let mut added = 0;
                for path in paths {
                    if self.excluded_tracks.insert(path) {
                        added += 1;
                    }
                }
                self.recommendations_dirty = true;
                self.home_cache_dirty = true;
                self.save_state();
                self.status = format!(
                    "Removed {added} track(s) from the library via {name}. Files were kept."
                );
            }
        }
        if let Some(index) = delete_playlist_files {
            if let Some(playlist) = self.playlists.get(index) {
                self.pending_bulk_delete = Some((
                    playlist.tracks.clone(),
                    format!("playlist ‘{}’", playlist.name),
                ));
            }
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
        if let Some(index) = copy_track_list {
            if let Some(playlist) = self.playlists.get(index) {
                let listing = playlist
                    .tracks
                    .iter()
                    .map(|path| path.to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("\n");
                ui.ctx().copy_text(listing);
                self.status = format!("Copied the track paths from {}.", playlist.name);
            }
        }
        if let Some(index) = remove_missing {
            if let Some(playlist) = self.playlists.get_mut(index) {
                let before = playlist.tracks.len();
                playlist.tracks.retain(|path| path.exists());
                let removed = before.saturating_sub(playlist.tracks.len());
                let name = playlist.name.clone();
                self.save_state();
                self.status = format!("Removed {removed} missing track(s) from {name}.");
            }
        }
        if let Some(index) = export {
            if let Some(playlist) = self.playlists.get(index) {
                let file_name = format!(
                    "{}.m3u8",
                    playlist
                        .name
                        .chars()
                        .map(|c| {
                            if ['<', '>', ';', ':', '\"', '/', '\\', '|', '?', '*'].contains(&c) {
                                '_'
                            } else {
                                c
                            }
                        })
                        .collect::<String>()
                );
                if let Some(destination) =
                    rfd::FileDialog::new().set_file_name(&file_name).save_file()
                {
                    let mut contents = String::from("#EXTM3U\n");
                    for path in &playlist.tracks {
                        contents.push_str(&path.to_string_lossy());
                        contents.push('\n');
                    }
                    match fs::write(&destination, contents) {
                        Ok(()) => {
                            self.status = format!("Exported playlist to {}.", destination.display())
                        }
                        Err(error) => self.status = format!("Could not export playlist: {error}"),
                    }
                }
            }
        }
        if let Some(index) = open {
            self.selected_playlist = Some(index);
            let tracks = self.indices_for_paths(&self.playlists[index].tracks.clone());
            if play || shuffle_play {
                if !tracks.is_empty() {
                    if shuffle_play {
                        self.play_shuffled_indices(tracks);
                    } else {
                        self.play_indices(tracks, 0);
                    }
                }
            } else if enqueue {
                self.add_tracks_to_queue(&tracks);
            } else if play_next {
                self.queue_tracks_next(&tracks);
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
                ui.label(RichText::new("Interface density").strong());
                ui.checkbox(&mut draft.compact_mode, "Compact controls and spacing");
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
                ui.label(RichText::new("Queue on startup").strong());
                ui.checkbox(&mut draft.show_queue_on_start, "Open Up next when Lilt starts");
                ui.end_row();
                ui.label(RichText::new("Default volume").strong());
                ui.add(
                    egui::Slider::new(&mut draft.volume, 0.0..=1.0)
                        .step_by(0.05)
                        .show_value(true)
                        .custom_formatter(|value, _| format!("{:.0}%", value * 100.0)),
                );
                ui.end_row();
                ui.label(RichText::new("Library scan speed").strong());
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::DragValue::new(&mut draft.scan_rate_per_second)
                                .range(0..=100_000)
                                .speed(250)
                                .suffix(" songs / sec"),
                        );
                        if ui.small_button("Default").clicked() {
                            draft.scan_rate_per_second = default_scan_rate_per_second();
                        }
                    });
                    ui.label(
                        RichText::new("Maximum scan speed in songs per second (default 5,000; files are read on several threads). Set to 0 for no limit. Applies to the next scan.")
                            .small()
                            .color(Color32::GRAY),
                    );
                });
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
                        self.home_cache_dirty = true;
                        self.save_state();
                        self.start_scan();
                    }
                });
                ui.end_row();
            });

        ui.add_space(14.0);
        ui.separator();
        ui.heading("Duplicate detector");
        ui.label(
            RichText::new("Liked songs are excluded from duplicate results. Like or unlike a song to refresh the list automatically.")
                .small()
                .color(Color32::GRAY),
        );
        ui.label(
            RichText::new("Duplicate checks run quietly in the background after every library scan. They never delete files automatically.")
                .small()
                .color(Color32::GRAY),
        );
        let mut duplicate_settings_changed = false;
        duplicate_settings_changed |= ui
            .checkbox(
                &mut self.duplicate_scan_enabled,
                "Enable background duplicate scanning",
            )
            .changed();
        ui.add_enabled_ui(self.duplicate_scan_enabled, |ui| {
            duplicate_settings_changed |= ui
                .checkbox(
                    &mut self.duplicate_check_exact,
                    "Bit-for-bit identical files",
                )
                .changed();
            duplicate_settings_changed |= ui
                .checkbox(
                    &mut self.duplicate_check_recordings,
                    "Same title, artist and runtime (different file/encode)",
                )
                .changed();
            duplicate_settings_changed |= ui
                .checkbox(
                    &mut self.duplicate_check_partial,
                    "Possible partial matches (more results, less certain)",
                )
                .changed();
        });
        let exact_extra_count = self
            .duplicate_groups
            .iter()
            .filter(|group| group.kind == DuplicateKind::BitForBit)
            .map(|group| group.tracks.len().saturating_sub(1))
            .sum::<usize>();
        let possible_group_count = self
            .duplicate_groups
            .iter()
            .filter(|group| group.kind != DuplicateKind::BitForBit)
            .count();
        ui.horizontal(|ui| {
            if self.duplicate_scan_busy {
                ui.spinner();
                ui.label("Checking duplicate files…");
            } else if !self.duplicate_scan_enabled {
                ui.label("Duplicate scanning is off.");
            } else if self.duplicate_groups.is_empty() {
                ui.label("No duplicate groups found in the latest scan.");
            } else {
                ui.label(format!(
                    "{} groups found · {} exact extra copies · {} possible matches",
                    self.duplicate_groups.len(),
                    exact_extra_count,
                    possible_group_count
                ));
            }
            if ui.button("Review results").clicked() {
                self.duplicate_review_open = true;
            }
            if ui
                .add_enabled(
                    self.duplicate_scan_enabled && !self.scan_busy,
                    egui::Button::new("↻ Check now"),
                )
                .clicked()
            {
                self.start_duplicate_scan();
            }
        });
        if exact_extra_count > 0 {
            ui.colored_label(
                Color32::from_rgb(255, 190, 120),
                format!(
                    "{} bit-for-bit extra copies can be deleted while keeping one copy per group.",
                    exact_extra_count
                ),
            );
        }
        if duplicate_settings_changed {
            self.save_state();
            if !self.duplicate_scan_enabled {
                if let Some(cancel) = self.duplicate_scan_cancel.take() {
                    cancel.store(true, Ordering::Release);
                }
                self.duplicate_scan_receiver = None;
                self.duplicate_scan_busy = false;
                self.duplicate_groups.clear();
                self.duplicate_notice = false;
            } else if !self.scan_busy {
                self.start_duplicate_scan();
            }
        }

        ui.add_space(16.0);
        ui.separator();
        ui.heading("Deleted song protection");
        ui.label(
            RichText::new(
                "Lilt remembers files you permanently delete. If the exact same file appears in a future scan, it is quietly removed again. Similar songs and different encodings are never auto-deleted.",
            )
            .small()
            .color(Color32::GRAY),
        );
        let mut auto_purge_changed = false;
        ui.add_enabled_ui(!self.scan_busy, |ui| {
            auto_purge_changed = ui
                .checkbox(
                    &mut self.auto_purge_deleted,
                    "Automatically remove previously deleted files when they reappear",
                )
                .changed();
        });
        if self.scan_busy {
            ui.label(
                RichText::new(
                    "Pause until the current library scan finishes to change deletion protection.",
                )
                .small()
                .color(Color32::GRAY),
            );
        }
        if auto_purge_changed {
            self.save_state();
            if self.auto_purge_deleted && !self.scan_busy {
                self.start_scan();
            }
        }
        let fingerprinted_count = self
            .deleted_songs
            .iter()
            .filter(|record| record.fingerprint.is_some())
            .count();
        ui.horizontal(|ui| {
            ui.label(format!(
                "{} remembered songs · {} with exact-file protection",
                self.deleted_songs.len(),
                fingerprinted_count
            ));
            if ui
                .add_enabled(
                    !self.deleted_songs.is_empty() && !self.scan_busy,
                    egui::Button::new("Clear history…"),
                )
                .clicked()
            {
                self.clear_deleted_history_confirmation = true;
            }
        });
        if self.deleted_songs.is_empty() {
            ui.label(
                RichText::new("Songs you permanently delete will appear here.")
                    .small()
                    .color(Color32::GRAY),
            );
        } else {
            let mut allow_again = None;
            egui::ScrollArea::vertical()
                .id_salt("deleted-song-history")
                .max_height(240.0)
                .show(ui, |ui| {
                    for (index, record) in self.deleted_songs.iter().enumerate().rev() {
                        egui::Frame::group(ui.style()).show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.label(RichText::new(&record.title).strong());
                                    let artist = record.artist.as_str();
                                    let artist = if is_unknown(artist) {
                                        "Unknown artist"
                                    } else {
                                        artist
                                    };
                                    let album = if is_unknown(&record.album) {
                                        "Unknown album"
                                    } else {
                                        record.album.as_str()
                                    };
                                    ui.label(
                                        RichText::new(format!(
                                            "{} · {} · {} · {}",
                                            artist,
                                            album,
                                            format_file_size(record.size_bytes),
                                            format_elapsed_since(record.deleted_at)
                                        ))
                                        .small()
                                        .color(Color32::GRAY),
                                    );
                                    ui.label(
                                        RichText::new(format!(
                                            "{} · {}",
                                            record.reason,
                                            record.original_path.display()
                                        ))
                                        .small()
                                        .color(Color32::GRAY),
                                    );
                                    if record.fingerprint.is_none() {
                                        ui.colored_label(
                                            Color32::from_rgb(255, 190, 120),
                                            "Exact fingerprint unavailable; this entry cannot auto-remove reappearing files.",
                                        );
                                    }
                                });
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui
                                            .add_enabled(!self.scan_busy, egui::Button::new("Allow again"))
                                            .clicked()
                                        {
                                            allow_again = Some(index);
                                        }
                                    },
                                );
                            });
                        });
                        ui.add_space(5.0);
                    }
                });
            if let Some(index) = allow_again {
                self.allow_deleted_song_again(index);
            }
        }

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
            self.queue_collapsed = !draft.show_queue_on_start;
            self.compact_mode = draft.compact_mode;
            self.discovery = draft.discovery;
            self.prefer_unplayed = draft.prefer_unplayed;
            self.recommendation_variety = draft.recommendation_variety.clamp(0.0, 1.0);
            self.recommendation_size = draft.recommendation_size.clamp(20, 100);
            self.ui_scale = draft.ui_scale.clamp(75.0, 150.0);
            self.volume = draft.volume.clamp(0.0, 1.0);
            self.scan_rate_per_second = draft.scan_rate_per_second.min(100_000);
            if let Some(sink) = &self.sink {
                sink.set_volume(self.volume);
            }
            self.recommendations_dirty = true;
            self.home_cache_dirty = true;
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
        self.mix_context().compute_recommendations()
    }

    fn load_ui_icons(&mut self, context: &egui::Context) {
        const ICONS: &[(&str, &[u8])] = &[
            (
                "play",
                include_bytes!("../assets/icons/ic-play-arrow-24px.svg"),
            ),
            ("pause", include_bytes!("../assets/icons/ic-pause-24px.svg")),
            (
                "previous",
                include_bytes!("../assets/icons/ic-skip-previous-24px.svg"),
            ),
            (
                "next",
                include_bytes!("../assets/icons/ic-skip-next-24px.svg"),
            ),
            (
                "favorite",
                include_bytes!("../assets/icons/ic-favorite-24px.svg"),
            ),
            (
                "favorite_border",
                include_bytes!("../assets/icons/ic-favorite-border-24px.svg"),
            ),
            (
                "shuffle",
                include_bytes!("../assets/icons/ic-shuffle-24px.svg"),
            ),
            (
                "repeat",
                include_bytes!("../assets/icons/ic-repeat-24px.svg"),
            ),
            (
                "repeat_one",
                include_bytes!("../assets/icons/ic-repeat-one-24px.svg"),
            ),
            (
                "volume_up",
                include_bytes!("../assets/icons/ic-volume-up-24px.svg"),
            ),
            (
                "volume_down",
                include_bytes!("../assets/icons/ic-volume-down-24px.svg"),
            ),
            (
                "volume_off",
                include_bytes!("../assets/icons/ic-volume-off-24px.svg"),
            ),
            ("home", include_bytes!("../assets/icons/ic-home-24px.svg")),
            (
                "songs",
                include_bytes!("../assets/icons/ic-library-music-24px.svg"),
            ),
            (
                "library_books",
                include_bytes!("../assets/icons/ic-library-books-24px.svg"),
            ),
            ("album", include_bytes!("../assets/icons/ic-album-24px.svg")),
            (
                "person",
                include_bytes!("../assets/icons/ic-person-24px.svg"),
            ),
            (
                "history",
                include_bytes!("../assets/icons/ic-history-24px.svg"),
            ),
            (
                "dislike",
                include_bytes!("../assets/icons/ic-thumb-down-24px.svg"),
            ),
            (
                "like",
                include_bytes!("../assets/icons/ic-thumb-up-24px.svg"),
            ),
            (
                "like_outline",
                include_bytes!("../assets/icons/ic-thumb-up-outline-24px.svg"),
            ),
            (
                "lyrics",
                include_bytes!("../assets/icons/ic-subtitles-24px.svg"),
            ),
            (
                "settings",
                include_bytes!("../assets/icons/ic-settings-24px.svg"),
            ),
            (
                "discover",
                include_bytes!("../assets/icons/ic-equalizer-24px.svg"),
            ),
        ];
        for (name, bytes) in ICONS {
            if self.ui_icons.contains_key(name) {
                continue;
            }
            let Ok(image) = egui_extras::image::load_svg_bytes(bytes) else {
                continue;
            };
            self.ui_icons.insert(
                *name,
                context.load_texture(
                    format!("ui-icon-{name}"),
                    image,
                    egui::TextureOptions::LINEAR,
                ),
            );
        }
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
        if !context.input(|input| input.focused) {
            return None;
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
                let result = image::load_from_memory(bytes.as_slice())
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
            if context.input(|input| input.focused) {
                context.request_repaint_after(Duration::from_millis(50));
            }
        }
        None
    }
}

impl eframe::App for PlayerApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let window_focused = ctx.input(|input| input.focused);
        self.load_ui_icons(ctx);
        #[cfg(target_os = "windows")]
        self.initialize_media_controls(frame, ctx);
        if window_focused {
            self.poll_scan();
            self.poll_home_job();
            self.poll_duplicate_scan();
            self.poll_artwork(ctx);
            self.poll_media_key_commands();
        }
        if !self.artwork_pending.is_empty() {
            if window_focused {
                ctx.request_repaint_after(Duration::from_millis(50));
            }
        }
        // Playback must keep advancing while the window is unfocused, even though
        // library scans, artwork, and other UI work stay paused until focus returns.
        if self.sink.as_ref().is_some_and(Sink::empty) {
            self.next_track();
        }
        #[cfg(target_os = "windows")]
        if window_focused {
            self.update_media_session();
        }
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
        let audio_is_playing = self
            .sink
            .as_ref()
            .is_some_and(|sink| !sink.is_paused() && !sink.empty());
        if window_focused && audio_is_playing {
            // A 10 FPS refresh keeps the compact equalizer/progress animation smooth
            // while focused. While unfocused, wake only at the estimated track end.
            ctx.request_repaint_after(Duration::from_millis(100));
        } else if audio_is_playing {
            // Wake once near the end of the current song so the queue can advance
            // without polling or refreshing the UI while the player is unfocused.
            let duration = self
                .current
                .and_then(|position| self.queue.get(position))
                .and_then(|&index| self.tracks.get(index))
                .map(|track| Duration::from_secs_f64(track.duration.max(0.0)));
            let position =
                self.playback_offset + self.sink.as_ref().map_or(Duration::ZERO, Sink::get_pos);
            let until_end = duration
                .map_or(Duration::from_secs(1), |duration| {
                    duration.saturating_sub(position)
                })
                .max(Duration::from_millis(50));
            ctx.request_repaint_after(until_end);
        } else if window_focused && (self.scan_busy || self.duplicate_scan_busy) {
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
            style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(42, 48, 76);
            style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(25, 26, 36);
            style.visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(14, 14, 19);
            style.visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(22, 23, 32);
            style.spacing.item_spacing = if self.compact_mode {
                Vec2::new(8.0, 6.0)
            } else {
                Vec2::new(12.0, 10.0)
            };
            style.spacing.button_padding = if self.compact_mode {
                Vec2::new(9.0, 6.0)
            } else {
                Vec2::new(12.0, 8.0)
            };
            style.spacing.interact_size = if self.compact_mode {
                Vec2::new(36.0, 34.0)
            } else {
                Vec2::new(42.0, 40.0)
            };
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

        if self.initial_library_loading {
            if window_focused {
                ctx.request_repaint_after(Duration::from_millis(160));
            }
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(Color32::from_rgb(10, 11, 17)))
                .show(ctx, |ui| {
                    ui.centered_and_justified(|ui| {
                        ui.vertical_centered(|ui| {
                            ui.add_space(12.0);
                            ui.add(egui::Spinner::new().size(32.0));
                            ui.add_space(18.0);
                            ui.heading("Preparing your library");
                            ui.label(
                                RichText::new("Lilt is importing your music in the background.")
                                    .color(Color32::GRAY),
                            );
                            ui.add_space(6.0);
                            ui.label(RichText::new(&self.scan_status).small().color(accent));
                        });
                    });
                });
            return;
        }

        egui::TopBottomPanel::top("top-bar").show(ctx, |ui| {
            let mut playlist_menu_action = None;
            ui.horizontal(|ui| {
                ui.heading(RichText::new("♫ LILT").color(accent).strong().size(22.0));
                ui.add_space(4.0);
                ui.menu_button("File", |ui| {
                    if ui.button("Add music folder…").clicked() {
                        self.add_folder();
                        ui.close_menu();
                    }
                    if ui.button("Rescan library").clicked() {
                        self.start_scan();
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Quit Lilt").clicked() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.menu_button("Playback", |ui| {
                    if ui.button("Play / Pause      Space").clicked() {
                        self.toggle_playback();
                        ui.close_menu();
                    }
                    if ui.button("Previous track      ←").clicked() {
                        self.previous_track();
                        ui.close_menu();
                    }
                    if ui.button("Next track      →").clicked() {
                        self.next_track();
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.checkbox(&mut self.shuffle, "Shuffle").changed() {
                        self.save_state();
                    }
                    ui.menu_button("Repeat", |ui| {
                        let mut changed = false;
                        changed |= ui
                            .selectable_value(&mut self.repeat, RepeatMode::Off, "Off")
                            .changed();
                        changed |= ui
                            .selectable_value(&mut self.repeat, RepeatMode::All, "All")
                            .changed();
                        changed |= ui
                            .selectable_value(&mut self.repeat, RepeatMode::One, "One")
                            .changed();
                        if changed {
                            self.save_state();
                            ui.close_menu();
                        }
                    });
                });
                ui.menu_button("View", |ui| {
                    for (page, label) in [
                        (Page::Home, "Home"),
                        (Page::Discover, "Discover"),
                        (Page::Songs, "Songs"),
                        (Page::Albums, "Albums"),
                        (Page::Artists, "Artists"),
                        (Page::Playlists, "Playlists"),
                        (Page::History, "History"),
                        (Page::Settings, "Settings"),
                    ] {
                        if ui.selectable_label(self.page == page, label).clicked() {
                            self.page = page;
                            if page != Page::Playlists {
                                self.selected_playlist = None;
                            }
                            ui.close_menu();
                        }
                    }
                    ui.separator();
                    if ui.button("Open Now Playing").clicked() {
                        self.now_playing_expanded = true;
                        ui.close_menu();
                    }
                    if ui
                        .button(if self.queue_collapsed {
                            "Show Up next"
                        } else {
                            "Hide Up next"
                        })
                        .clicked()
                    {
                        self.queue_collapsed = !self.queue_collapsed;
                        self.save_state();
                        ui.close_menu();
                    }
                });
                let playlist_menu_items = self
                    .playlists
                    .iter()
                    .enumerate()
                    .map(|(index, playlist)| (index, playlist.name.clone(), playlist.tracks.len()))
                    .collect::<Vec<_>>();
                ui.menu_button("Playlists", |ui| {
                    if ui.button("Open playlist library…").clicked() {
                        self.page = Page::Playlists;
                        self.selected_playlist = None;
                        ui.close_menu();
                    }
                    ui.separator();
                    if playlist_menu_items.is_empty() {
                        ui.label("No playlists yet");
                    } else {
                        egui::ScrollArea::vertical()
                            .max_height(320.0)
                            .show(ui, |ui| {
                                for (index, name, count) in &playlist_menu_items {
                                    ui.horizontal(|ui| {
                                        if ui
                                            .button(format!("{name} · {count}"))
                                            .on_hover_text("Open this playlist")
                                            .clicked()
                                        {
                                            self.page = Page::Playlists;
                                            self.selected_playlist = Some(*index);
                                            ui.close_menu();
                                        }
                                        if ui.small_button("Play").clicked() {
                                            playlist_menu_action = Some((*index, false));
                                            ui.close_menu();
                                        }
                                        if ui.small_button("Shuffle").clicked() {
                                            playlist_menu_action = Some((*index, true));
                                            ui.close_menu();
                                        }
                                    });
                                }
                            });
                    }
                });
                ui.menu_button("Help", |ui| {
                    ui.label(RichText::new("Keyboard shortcuts").strong());
                    ui.label("Space or Ctrl+P  ·  Play / pause");
                    ui.label("← / →  ·  Previous / next track");
                    ui.label("Ctrl+F or /  ·  Search");
                    ui.separator();
                    ui.label(format!("Lilt Player {}", env!("CARGO_PKG_VERSION")));
                });
                ui.add_space(4.0);
                ui.add_sized(
                    [300.0, 34.0],
                    egui::TextEdit::singleline(&mut self.query)
                        .id(egui::Id::new("global-search"))
                        .hint_text("Search songs, artists, albums…"),
                );
                if ui
                    .selectable_label(
                        !self.queue_collapsed,
                        format!("♫ Queue  {}", self.queue.len()),
                    )
                    .clicked()
                {
                    self.queue_collapsed = !self.queue_collapsed;
                    self.save_state();
                }
                if ui
                    .selectable_label(self.page == Page::Settings, "⚙ Settings")
                    .clicked()
                {
                    self.page = Page::Settings;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.scan_busy {
                        ui.spinner();
                        let pulse_phase = ctx.input(|input| input.time) as f32;
                        let pulse = 0.62 + (pulse_phase * 3.0).sin().abs() * 0.38;
                        ui.label(
                            RichText::new(&self.scan_status)
                                .small()
                                .color(accent.linear_multiply(pulse)),
                        );
                    } else {
                        ui.label(
                            RichText::new(&self.scan_status)
                                .small()
                                .color(Color32::GRAY),
                        );
                    }
                });
            });
            if let Some((index, shuffle)) = playlist_menu_action {
                let tracks = self
                    .playlists
                    .get(index)
                    .map(|playlist| self.indices_for_paths(&playlist.tracks))
                    .unwrap_or_default();
                self.page = Page::Playlists;
                self.selected_playlist = Some(index);
                if tracks.is_empty() {
                    self.status = "This playlist has no available tracks to play.".into();
                } else if shuffle {
                    self.play_shuffled_indices(tracks);
                } else {
                    self.play_indices(tracks, 0);
                }
            }
        });

        egui::SidePanel::left("navigation")
            .resizable(false)
            .default_width(176.0)
            .show(ctx, |ui| {
                ui.add_space(12.0);
                egui::ScrollArea::vertical()
                    .id_salt("navigation-items")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for (label, items) in [
                            (
                                "DISCOVER",
                                vec![
                                    (Page::Home, "Home", "home"),
                                    (Page::Discover, "Discover", "discover"),
                                    (Page::Songs, "Songs", "songs"),
                                    (Page::Albums, "Albums", "album"),
                                    (Page::Artists, "Artists", "person"),
                                ],
                            ),
                            (
                                "YOUR MUSIC",
                                vec![
                                    (Page::LikedSongs, "Liked music", "like"),
                                    (Page::LikedArtists, "Liked artists", "person"),
                                    (Page::LikedAlbums, "Saved albums", "favorite"),
                                ],
                            ),
                            (
                                "ACTIVITY",
                                vec![
                                    (Page::History, "History", "history"),
                                    (Page::Disliked, "Disliked", "dislike"),
                                    (Page::Lyrics, "Lyrics", "lyrics"),
                                    (Page::Settings, "Settings", "settings"),
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
                            for (page, title, icon) in items {
                                if animated_nav_item(
                                    ui,
                                    &format!("{label}-{title}"),
                                    title,
                                    self.ui_icons.get(icon),
                                    self.page == page,
                                    accent,
                                ) {
                                    self.page = page;
                                    self.selected_playlist = None;
                                }
                            }
                            ui.add_space(8.0);
                        }
                        ui.separator();
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("PLAYLISTS")
                                    .small()
                                    .strong()
                                    .color(Color32::from_rgb(150, 145, 151)),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui
                                        .add_sized([34.0, 32.0], egui::Button::new("＋"))
                                        .on_hover_text("Create a playlist")
                                        .clicked()
                                    {
                                        self.page = Page::Playlists;
                                        self.selected_playlist = None;
                                    }
                                },
                            );
                        });
                        let playlist_names = self
                            .playlists
                            .iter()
                            .map(|playlist| playlist.name.clone())
                            .collect::<Vec<_>>();
                        for (index, playlist_name) in playlist_names.iter().enumerate() {
                            let selected = self.page == Page::Playlists
                                && self.selected_playlist == Some(index);
                            if animated_nav_item(
                                ui,
                                &format!("playlist-{index}"),
                                playlist_name,
                                self.ui_icons.get("library_books"),
                                selected,
                                accent,
                            ) {
                                self.page = Page::Playlists;
                                self.selected_playlist = Some(index);
                            }
                        }
                        ui.add_space(8.0);
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

        if !self.queue_collapsed && !self.now_playing_expanded {
            egui::SidePanel::right("up-next")
                .resizable(true)
                .default_width(310.0)
                .width_range(260.0..=420.0)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        if ui
                            .add_sized([38.0, 38.0], egui::Button::new("×"))
                            .on_hover_text("Collapse Up next")
                            .clicked()
                        {
                            self.queue_collapsed = true;
                            self.save_state();
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
                                track.path.clone(),
                            )
                        });
                        if let Some((title, artist, album, current_path)) = details {
                            let current_liked = self.liked_songs.contains(&current_path);
                            let current_disliked = self.disliked_songs.contains(&current_path);
                            let mut restart_now_playing = false;
                            let mut skip_now_playing = false;
                            let mut like_now_playing = false;
                            let mut dislike_now_playing = false;
                            let mut remove_now_playing_from_library = false;
                            let mut delete_now_playing_file = false;

                            let card = egui::Frame::new()
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
                                                egui::Label::new(RichText::new(&title).strong())
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
                            let card_response = ui.interact(
                                card.response.rect,
                                ui.make_persistent_id(("up-next-now-playing", track_index)),
                                egui::Sense::click(),
                            );
                            card_response
                                .on_hover_text("Right-click for current track actions")
                                .context_menu(|ui| {
                                    ui.label(RichText::new(&title).strong());
                                    ui.label(RichText::new(&artist).small().color(Color32::GRAY));
                                    ui.separator();
                                    if ui.button("↻  Restart this track").clicked() {
                                        restart_now_playing = true;
                                        ui.close_menu();
                                    }
                                    if ui.button("⏭  Skip to next track").clicked() {
                                        skip_now_playing = true;
                                        ui.close_menu();
                                    }
                                    ui.separator();
                                    if ui.button(if current_liked { "♥  Unlike song" } else { "♡  Like song" }).clicked() {
                                        like_now_playing = true;
                                        ui.close_menu();
                                    }
                                    if ui.button(if current_disliked { "−  Remove dislike" } else { "−  Dislike song" }).clicked() {
                                        dislike_now_playing = true;
                                        ui.close_menu();
                                    }
                                    ui.separator();
                                    if ui.button("▤  Copy song title and artist").clicked() {
                                        ui.ctx().copy_text(format!("{} — {}", title, artist));
                                        ui.close_menu();
                                    }
                                    if ui.button("▣  Copy file path").clicked() {
                                        ui.ctx().copy_text(current_path.to_string_lossy().into_owned());
                                        ui.close_menu();
                                    }
                                    #[cfg(target_os = "windows")]
                                    if ui.button("📂  Show in File Explorer").clicked() {
                                        let _ = std::process::Command::new("explorer.exe")
                                            .arg(format!("/select,{}", current_path.display()))
                                            .spawn();
                                        ui.close_menu();
                                    }
                                    ui.separator();
                                    if ui.button("Remove from library (keep file)").clicked() {
                                        remove_now_playing_from_library = true;
                                        ui.close_menu();
                                    }
                                    if ui.button(RichText::new("Delete audio file permanently…").color(Color32::LIGHT_RED)).clicked() {
                                        delete_now_playing_file = true;
                                        ui.close_menu();
                                    }
                                });

                            if restart_now_playing {
                                if let Some(position) = current_position {
                                    self.play_queue_at(position);
                                }
                            }
                            if skip_now_playing {
                                self.next_track();
                            }
                            if like_now_playing {
                                self.toggle_song_reaction(track_index, true);
                            }
                            if dislike_now_playing {
                                self.toggle_song_reaction(track_index, false);
                            }
                            if remove_now_playing_from_library {
                                self.excluded_tracks.insert(current_path.clone());
                                self.recommendations_dirty = true;
                                self.home_cache_dirty = true;
                                self.save_state();
                                self.status = format!("Removed {title} from the library. The file was kept.");
                            }
                            if delete_now_playing_file {
                                self.pending_delete = Some((current_path, title));
                            }
                        }
                    }
                    let upcoming_start = current_position.map_or(0, |position| position + 1);
                    let upcoming_count = self.queue.len().saturating_sub(upcoming_start);
                    ui.add_space(8.0);
                    // These action flags must live outside the UI closures so their
                    // values can be applied after the queue panel has been drawn.
                    let mut clear_upcoming = false;
                    let mut delete_all_upcoming_files = false;
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("COMING UP").small().strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{upcoming_count} tracks"))
                                    .small()
                                    .color(Color32::GRAY),
                            );
                            if upcoming_count > 0 && ui.small_button("Clear all").on_hover_text("Remove every upcoming song from the queue").clicked() {
                                clear_upcoming = true;
                            }
                            if upcoming_count > 0 && ui.small_button("Delete all…").on_hover_text("Permanently delete every upcoming audio file after confirmation").clicked() {
                                delete_all_upcoming_files = true;
                            }
                        });
                    });
                    let mut play_queue_position = None;
                    let mut remove_queue_position = None;
                    let mut move_queue = None;
                    let mut queue_like = None;
                    let mut queue_dislike = None;
                    let mut queue_delete_file = None;
                    let mut queue_remove_from_library = None;
                    egui::ScrollArea::vertical()
                        .id_salt("up-next-tracks")
                        .auto_shrink([false, false])
                        .show_rows(ui, 72.0, upcoming_count, |ui, row_range| {
                            for row in row_range {
                                let position = upcoming_start + row;
                                let track_index = self.queue[position];
                                // Copy the row data before requesting artwork. artwork_texture
                                // mutably borrows the app, so no reference into self.tracks may
                                // remain live across that call.
                                let Some((track_title, track_artist, queue_track_path)) = self
                                    .tracks
                                    .get(track_index)
                                    .map(|track| {
                                        (
                                            track.title.clone(),
                                            display_credit(&track.artist),
                                            track.path.clone(),
                                        )
                                    })
                                else {
                                    continue;
                                };
                                let queue_track_liked = self.liked_songs.contains(&queue_track_path);
                                let queue_track_disliked = self.disliked_songs.contains(&queue_track_path);
                                let cover = self.artwork_texture(ui.ctx(), track_index);
                                ui.add_space(5.0);
                                let row_response = egui::Frame::new()
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
                                                    egui::Label::new(
                                                        RichText::new("♫")
                                                            .color(accent_color(&self.accent))
                                                            .size(24.0),
                                                    ),
                                                );
                                            }
                                            // The row has no inline action buttons; song actions
                                            // are available from its right-click context menu.
                                            let text_width = (ui.available_width() - 8.0).max(50.0);
                                            ui.vertical(|ui| {
                                                ui.add_sized(
                                                    [text_width, 24.0],
                                                    egui::Label::new(
                                                        RichText::new(&track_title).strong(),
                                                    )
                                                    .truncate(),
                                                );
                                                ui.add_sized(
                                                    [text_width, 20.0],
                                                    egui::Label::new(
                                                        RichText::new(track_artist.clone())
                                                            .small()
                                                            .color(Color32::GRAY),
                                                    )
                                                    .truncate(),
                                                );
                                            });
                                        });
                                    });
                                // Make the complete row the interaction surface: left-click
                                // plays it, while right-click opens the full action menu.
                                let row_response = ui.interact(
                                    row_response.response.rect,
                                    ui.make_persistent_id(("up-next-row", position)),
                                    egui::Sense::click(),
                                );
                                if row_response.clicked() {
                                    play_queue_position = Some(position);
                                }
                                row_response
                                    .on_hover_text("Click to play · right-click for all song actions")
                                    .context_menu(|ui| {
                                        ui.label(RichText::new(&track_title).strong());
                                        ui.label(RichText::new(&track_artist).small().color(Color32::GRAY));
                                        ui.separator();
                                        if ui.button("▶  Play this track").clicked() {
                                            play_queue_position = Some(position);
                                            ui.close_menu();
                                        }
                                        if ui.button("↑  Move to top of queue").clicked() {
                                            move_queue = Some((position, upcoming_start));
                                            ui.close_menu();
                                        }
                                        if ui.button("↓  Move to end of queue").clicked() {
                                            move_queue = Some((position, self.queue.len().saturating_sub(1)));
                                            ui.close_menu();
                                        }
                                        if ui.button("×  Remove from queue").clicked() {
                                            remove_queue_position = Some(position);
                                            ui.close_menu();
                                        }
                                        ui.separator();
                                        if ui.button(if queue_track_liked { "♥  Unlike song" } else { "♡  Like song" }).clicked() {
                                            queue_like = Some(track_index);
                                            ui.close_menu();
                                        }
                                        if ui.button(if queue_track_disliked { "−  Remove dislike" } else { "−  Dislike song" }).clicked() {
                                            queue_dislike = Some(track_index);
                                            ui.close_menu();
                                        }
                                        if ui.button("▤  Copy song title and artist").clicked() {
                                            ui.ctx().copy_text(format!("{} — {}", track_title, track_artist));
                                            ui.close_menu();
                                        }
                                        if ui.button("▣  Copy file path").clicked() {
                                            ui.ctx().copy_text(queue_track_path.to_string_lossy().into_owned());
                                            ui.close_menu();
                                        }
                                        ui.separator();
                                        if ui.button("Remove from library (keep file)").clicked() {
                                            queue_remove_from_library = Some((queue_track_path.clone(), track_title.clone()));
                                            ui.close_menu();
                                        }
                                        if ui.button(RichText::new("Delete audio file permanently…").color(Color32::LIGHT_RED)).clicked() {
                                            queue_delete_file = Some((queue_track_path.clone(), track_title.clone()));
                                            ui.close_menu();
                                        }
                                    });
                            }
                        });
                    if clear_upcoming {
                        self.queue.truncate(upcoming_start);
                        self.status = format!("Cleared {upcoming_count} upcoming track(s) from the queue.");
                    }
                    if delete_all_upcoming_files {
                        let paths = self.queue.iter().skip(upcoming_start)
                            .filter_map(|&index| self.tracks.get(index).map(|track| track.path.clone()))
                            .collect::<Vec<_>>();
                        self.pending_bulk_delete = Some((paths, "up next queue".into()));
                    }
                    if let Some(index) = queue_like { self.toggle_song_reaction(index, true); }
                    if let Some(index) = queue_dislike { self.toggle_song_reaction(index, false); }
                    if let Some((path, title)) = queue_remove_from_library {
                        self.excluded_tracks.insert(path);
                        self.recommendations_dirty = true;
                        self.home_cache_dirty = true;
                        self.save_state();
                        self.status = format!("Removed {title} from the library. The file was kept.");
                    }
                    if let Some((path, title)) = queue_delete_file { self.pending_delete = Some((path, title)); }
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
            .exact_height(88.0)
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
                        let current_track = current_track_index
                            .and_then(|index| self.tracks.get(index))
                            .cloned();
                        let current_cover =
                            current_track_index.and_then(|index| self.artwork_texture(ctx, index));
                        let current_track_liked = current_track_index
                            .and_then(|index| self.tracks.get(index))
                            .is_some_and(|track| self.liked_songs.contains(&track.path));
                        let current_track_disliked = current_track_index
                            .and_then(|index| self.tracks.get(index))
                            .is_some_and(|track| self.disliked_songs.contains(&track.path));
                        let mut like_current = false;
                        let mut dislike_current = false;
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
                            let played_color = accent;
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
                                let (cover_rect, cover_response) =
                                    ui.allocate_exact_size(Vec2::splat(48.0), egui::Sense::click());
                                paint_cover_or_placeholder(
                                    ui,
                                    current_cover.as_ref(),
                                    cover_rect,
                                    current_track_index.unwrap_or(0),
                                );
                                if cover_response.clicked() {
                                    self.now_playing_expanded = !self.now_playing_expanded;
                                }
                                cover_response.on_hover_text("Open the expanded player");
                                ui.vertical(|ui| {
                                    let title = current_track
                                        .as_ref()
                                        .map_or("Nothing playing", |track| track.title.as_str());
                                    ui.add_sized(
                                        [190.0, 22.0],
                                        egui::Label::new(RichText::new(title).strong()).truncate(),
                                    );
                                    let artist = current_track.as_ref().map_or_else(
                                        || "Choose a mix or song".to_string(),
                                        |track| display_credit(&track.artist),
                                    );
                                    ui.add_sized(
                                        [190.0, 18.0],
                                        egui::Label::new(
                                            RichText::new(artist).small().color(Color32::GRAY),
                                        )
                                        .truncate(),
                                    );
                                });
                                if ui
                                    .add_sized(
                                        [36.0, 34.0],
                                        egui::Button::image(
                                            egui::Image::new(&self.ui_icons["previous"])
                                                .fit_to_exact_size(Vec2::splat(22.0)),
                                        ),
                                    )
                                    .on_hover_text("Previous track (Left Arrow)")
                                    .clicked()
                                {
                                    self.previous_track();
                                }
                                let playing =
                                    self.sink.as_ref().is_some_and(|sink| !sink.is_paused());
                                let play_animation = ctx.animate_bool_with_time(
                                    egui::Id::new("player-play-animation"),
                                    playing,
                                    0.18,
                                );
                                if ui
                                    .add(
                                        egui::Button::image(
                                            egui::Image::new(if playing {
                                                &self.ui_icons["pause"]
                                            } else {
                                                &self.ui_icons["play"]
                                            })
                                            .fit_to_exact_size(Vec2::splat(24.0)),
                                        )
                                        .fill(accent)
                                        .min_size(Vec2::new(62.0 + play_animation * 4.0, 34.0)),
                                    )
                                    .on_hover_text("Play or pause (Space or Ctrl+P)")
                                    .clicked()
                                {
                                    self.toggle_playback();
                                }
                                if ui
                                    .add_sized(
                                        [36.0, 34.0],
                                        egui::Button::image(
                                            egui::Image::new(&self.ui_icons["next"])
                                                .fit_to_exact_size(Vec2::splat(22.0)),
                                        ),
                                    )
                                    .on_hover_text("Next track (Right Arrow)")
                                    .clicked()
                                {
                                    self.next_track();
                                }
                                if ui
                                    .add_enabled(
                                        current_track_index.is_some(),
                                        egui::Button::image(
                                            egui::Image::new(if current_track_liked {
                                                &self.ui_icons["like"]
                                            } else {
                                                &self.ui_icons["like_outline"]
                                            })
                                            .fit_to_exact_size(Vec2::splat(20.0)),
                                        )
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
                                if ui
                                    .add_enabled(
                                        current_track_index.is_some(),
                                        egui::Button::image(
                                            egui::Image::new(&self.ui_icons["dislike"])
                                                .fit_to_exact_size(Vec2::splat(20.0)),
                                        )
                                        .selected(current_track_disliked),
                                    )
                                    .on_hover_text(if current_track_disliked {
                                        "Remove dislike"
                                    } else {
                                        "Dislike this song"
                                    })
                                    .clicked()
                                {
                                    dislike_current = true;
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
                                    .add(
                                        egui::Button::image(
                                            egui::Image::new(&self.ui_icons["shuffle"])
                                                .fit_to_exact_size(Vec2::splat(18.0)),
                                        )
                                        .selected(self.shuffle),
                                    )
                                    .on_hover_text("Shuffle")
                                    .clicked()
                                {
                                    self.shuffle = !self.shuffle;
                                    self.save_state();
                                }
                                let repeat_icon = if self.repeat == RepeatMode::One {
                                    "repeat_one"
                                } else {
                                    "repeat"
                                };
                                if ui
                                    .add(
                                        egui::Button::image(
                                            egui::Image::new(&self.ui_icons[repeat_icon])
                                                .fit_to_exact_size(Vec2::splat(18.0)),
                                        )
                                        .selected(self.repeat != RepeatMode::Off),
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
                                ui.add(
                                    egui::Image::new(&self.ui_icons["volume_up"])
                                        .fit_to_exact_size(Vec2::splat(17.0)),
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
                                if ui.button("Queue").clicked() {
                                    self.queue_collapsed = !self.queue_collapsed;
                                }
                                if ui.button("Expand").clicked() {
                                    self.now_playing_expanded = true;
                                }
                            });
                        });
                        if like_current {
                            if let Some(index) = current_track_index {
                                self.toggle_song_reaction(index, true);
                            }
                        }
                        if dislike_current {
                            if let Some(index) = current_track_index {
                                self.toggle_song_reaction(index, false);
                            }
                        }
                    });
                ui.add_space(6.0);
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            if self.now_playing_expanded {
                self.show_now_playing_view(ui);
            } else {
                self.show_library(ui);
            }
        });

        self.show_properties_window(ctx);

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

        if let Some((paths, description)) = self.pending_bulk_delete.clone() {
            let mut confirm = false;
            let mut cancel = false;
            egui::Window::new("Delete selected audio files?")
                .id(egui::Id::new("confirm-context-bulk-delete"))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label(RichText::new(format!("Permanently delete {} audio file(s) from {description}?", paths.len())).strong());
                    ui.colored_label(Color32::LIGHT_RED, "This deletes the actual files from disk and cannot be undone. Lilt will remember these deletions to prevent accidental re-import.");
                    for path in paths.iter().take(5) {
                        ui.label(RichText::new(path.display().to_string()).small().color(Color32::GRAY));
                    }
                    if paths.len() > 5 {
                        ui.label(RichText::new(format!("…and {} more", paths.len() - 5)).small().color(Color32::GRAY));
                    }
                    ui.horizontal(|ui| {
                        confirm = ui.add_sized([164.0, 40.0], egui::Button::new("Delete all permanently")).clicked();
                        cancel = ui.add_sized([100.0, 40.0], egui::Button::new("Cancel")).clicked();
                    });
                });
            if confirm {
                self.pending_bulk_delete = None;
                self.delete_audio_files_permanently(
                    &paths,
                    &format!("Bulk deletion: {description}"),
                    &description,
                );
            } else if cancel {
                self.pending_bulk_delete = None;
            }
        }

        if self.duplicate_notice {
            let exact_extra_count = self
                .duplicate_groups
                .iter()
                .filter(|group| group.kind == DuplicateKind::BitForBit)
                .map(|group| group.tracks.len().saturating_sub(1))
                .sum::<usize>();
            let possible_groups = self
                .duplicate_groups
                .iter()
                .filter(|group| group.kind != DuplicateKind::BitForBit)
                .count();
            let mut notice_open = self.duplicate_notice;
            let mut dismiss_notice = false;
            let mut open_review = false;
            egui::Window::new("Duplicate scan complete")
                .id(egui::Id::new("duplicate-scan-notice"))
                .open(&mut notice_open)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_TOP, Vec2::new(0.0, 76.0))
                .show(ctx, |ui| {
                    ui.label(
                        RichText::new("Lilt found possible duplicate songs in your library.")
                            .strong(),
                    );
                    ui.label(format!(
                        "{} exact extra copies · {} possible-match groups",
                        exact_extra_count, possible_groups
                    ));
                    ui.horizontal(|ui| {
                        open_review = ui
                            .add_sized([160.0, 38.0], egui::Button::new("Review duplicates"))
                            .clicked();
                        if ui.small_button("Later").clicked() {
                            dismiss_notice = true;
                        }
                    });
                });
            self.duplicate_notice = notice_open && !dismiss_notice && !open_review;
            if open_review {
                self.duplicate_review_open = true;
            }
        }

        if self.duplicate_review_open {
            let groups = self.duplicate_groups.clone();
            let mut review_open = self.duplicate_review_open;
            let mut delete_paths: Option<Vec<PathBuf>> = None;
            let mut hide_paths: Option<Vec<PathBuf>> = None;
            egui::Window::new("Duplicate manager")
                .id(egui::Id::new("duplicate-manager"))
                .open(&mut review_open)
                .default_size(Vec2::new(760.0, 620.0))
                .resizable(true)
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.label(
                        RichText::new("One file in each group is marked KEEP. Lilt never removes files without your confirmation.")
                            .small()
                            .color(Color32::GRAY),
                    );
                    if groups.is_empty() {
                        ui.add_space(12.0);
                        ui.label("No duplicate groups found. Run a new library scan to check again.");
                    } else {
                        let exact_extras = groups
                            .iter()
                            .filter(|group| group.kind == DuplicateKind::BitForBit)
                            .flat_map(|group| group.tracks.iter().skip(1).map(|track| track.path.clone()))
                            .collect::<Vec<_>>();
                        let suggested_extras = groups
                            .iter()
                            .filter(|group| group.kind != DuplicateKind::BitForBit)
                            .flat_map(|group| group.tracks.iter().skip(1).map(|track| track.path.clone()))
                            .collect::<Vec<_>>();
                        ui.horizontal_wrapped(|ui| {
                            if !exact_extras.is_empty()
                                && ui.button(format!("Delete all {} exact extra copies…", exact_extras.len())).clicked()
                            {
                                delete_paths = Some(exact_extras.clone());
                            }
                            if !suggested_extras.is_empty()
                                && ui.button(format!("Remove {} possible matches from library", suggested_extras.len())).clicked()
                            {
                                hide_paths = Some(suggested_extras.clone());
                            }
                        });
                        ui.separator();
                        egui::ScrollArea::vertical()
                            .id_salt("duplicate-review-list")
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                for (group_index, group) in groups.iter().enumerate() {
                                    let mut group_delete = false;
                                    let mut group_hide = false;
                                    egui::Frame::group(ui.style()).show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            ui.heading(group.kind.title());
                                            ui.label(RichText::new(format!("{} files", group.tracks.len())).small().color(Color32::GRAY));
                                        });
                                        ui.label(
                                            RichText::new(group.kind.description())
                                                .small()
                                                .color(Color32::GRAY),
                                        );
                                        ui.add_space(4.0);
                                        for (track_index, track) in group.tracks.iter().enumerate() {
                                            ui.horizontal(|ui| {
                                                ui.label(RichText::new(if track_index == 0 { "KEEP" } else { "COPY" })
                                                    .strong()
                                                    .color(if track_index == 0 {
                                                        Color32::from_rgb(145, 220, 165)
                                                    } else {
                                                        Color32::from_rgb(255, 210, 125)
                                                    }));
                                                ui.vertical(|ui| {
                                                    ui.label(RichText::new(&track.title).strong());
                                                    ui.label(RichText::new(format!(
                                                        "{} · {} · {} · {}",
                                                        display_credit(&track.artist),
                                                        format_time(track.duration),
                                                        format_file_size(track.size_bytes),
                                                        track.path.display()
                                                    )).small().color(Color32::GRAY));
                                                });
                                            });
                                        }
                                        ui.horizontal(|ui| {
                                            if group.kind == DuplicateKind::BitForBit {
                                                group_delete = ui.button("Delete extra copies…").clicked();
                                            } else {
                                                group_hide = ui.button("Remove extras from library").clicked();
                                            }
                                        });
                                    });
                                    if group_delete {
                                        delete_paths = Some(group.tracks.iter().skip(1).map(|track| track.path.clone()).collect());
                                    }
                                    if group_hide {
                                        hide_paths = Some(group.tracks.iter().skip(1).map(|track| track.path.clone()).collect());
                                    }
                                    if group_index + 1 < groups.len() {
                                        ui.add_space(8.0);
                                    }
                                }
                            });
                    }
                });
            self.duplicate_review_open = review_open;
            if let Some(paths) = delete_paths {
                self.pending_duplicate_delete = Some(paths);
            }
            if let Some(paths) = hide_paths {
                self.remove_duplicate_suggestions_from_library(&paths);
                self.duplicate_review_open = false;
            }
        }

        if let Some(paths) = self.pending_duplicate_delete.clone() {
            let mut confirm = false;
            let mut cancel = false;
            egui::Window::new("Delete exact duplicate copies?")
                .id(egui::Id::new("confirm-duplicate-delete"))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label(RichText::new(format!("Permanently delete {} bit-for-bit duplicate file(s)?", paths.len())).strong());
                    ui.colored_label(Color32::LIGHT_RED, "The files will be removed from disk. One copy per duplicate group is kept. This cannot be undone.");
                    for path in paths.iter().take(4) {
                        ui.label(RichText::new(path.display().to_string()).small().color(Color32::GRAY));
                    }
                    if paths.len() > 4 {
                        ui.label(RichText::new(format!("…and {} more", paths.len() - 4)).small().color(Color32::GRAY));
                    }
                    ui.horizontal(|ui| {
                        confirm = ui.add_sized([160.0, 40.0], egui::Button::new("Delete permanently")).clicked();
                        cancel = ui.add_sized([100.0, 40.0], egui::Button::new("Cancel")).clicked();
                    });
                });
            if confirm {
                self.pending_duplicate_delete = None;
                self.delete_duplicate_files(&paths);
            } else if cancel {
                self.pending_duplicate_delete = None;
            }
        }

        if self.clear_deleted_history_confirmation {
            let mut confirm = false;
            let mut cancel = false;
            egui::Window::new("Clear deleted-song history?")
                .id(egui::Id::new("confirm-clear-deleted-history"))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label(RichText::new("Remove all remembered deletion rules?").strong());
                    ui.colored_label(
                        Color32::LIGHT_RED,
                        "This will stop Lilt from automatically removing those exact files if they appear again. It will not restore files already deleted.",
                    );
                    ui.horizontal(|ui| {
                        confirm = ui
                            .add_enabled(!self.scan_busy, egui::Button::new("Clear history"))
                            .clicked();
                        cancel = ui.button("Cancel").clicked();
                    });
                });
            if confirm {
                self.deleted_songs.clear();
                self.clear_deleted_history_confirmation = false;
                if self.save_state() {
                    self.status = "Deleted-song history cleared.".into();
                }
            } else if cancel {
                self.clear_deleted_history_confirmation = false;
            }
        }
    }
}

impl MixContext {
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
                let artist_key = if is_unknown(&track.artist) {
                    String::new()
                } else {
                    track.artist.to_ascii_lowercase()
                };
                let album_key = if is_unknown(&track.album) {
                    String::new()
                } else {
                    track.album.to_ascii_lowercase()
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
                    artist_key,
                    album_key,
                )
            })
            .collect::<Vec<_>>();

        // Variety penalties used to rescan the whole selected list for every
        // remaining candidate. These maps preserve the same scoring while
        // avoiding millions of repeated string comparisons on large libraries.
        let mut recommendations =
            Vec::<usize>::with_capacity(self.recommendation_size.clamp(20, 100));
        let mut selected_artist_counts = HashMap::<String, usize>::new();
        let mut selected_album_counts = HashMap::<String, usize>::new();
        while !remaining.is_empty()
            && recommendations.len() < self.recommendation_size.clamp(20, 100)
        {
            let (best_position, _) = remaining
                .iter()
                .enumerate()
                .map(|(position, (_, score, artist_key, album_key))| {
                    let repeated_artist =
                        selected_artist_counts.get(artist_key).copied().unwrap_or(0) as f32;
                    let repeated_album =
                        selected_album_counts.get(album_key).copied().unwrap_or(0) as f32;
                    let variety_penalty = repeated_artist * 1.8 * self.recommendation_variety
                        + repeated_album * 2.5 * self.recommendation_variety;
                    (position, *score - variety_penalty)
                })
                .max_by(|left, right| {
                    left.1
                        .total_cmp(&right.1)
                        .then_with(|| right.0.cmp(&left.0))
                })
                .expect("remaining candidates are non-empty");
            let (index, _, artist_key, album_key) = remaining.swap_remove(best_position);
            recommendations.push(index);
            if !artist_key.is_empty() {
                *selected_artist_counts.entry(artist_key).or_default() += 1;
            }
            if !album_key.is_empty() {
                *selected_album_counts.entry(album_key).or_default() += 1;
            }
        }
        recommendations
    }

    fn build_daily_mixes(&self, seed: u64) -> [Vec<usize>; 3] {
        let candidates = self.home_track_indices();
        let mut mixes: [Vec<usize>; 3] = std::array::from_fn(|_| Vec::new());
        if candidates.is_empty() {
            return mixes;
        }

        let ranked = self.compute_recommendations();
        let rank_by_track = ranked
            .iter()
            .enumerate()
            .map(|(rank, &index)| (index, rank))
            .collect::<HashMap<_, _>>();
        let played_rank = self
            .history
            .iter()
            .enumerate()
            .map(|(rank, path)| (path.clone(), rank))
            .collect::<HashMap<_, _>>();
        let mut genre_frequency = HashMap::<String, usize>::new();
        for &index in &candidates {
            let genre = self.tracks[index].genre.trim().to_lowercase();
            if !is_unknown(&genre) {
                *genre_frequency.entry(genre).or_default() += 1;
            }
        }
        let newest_added = candidates
            .iter()
            .map(|&index| self.tracks[index].added)
            .max()
            .unwrap_or_default();
        let target_size = self
            .recommendation_size
            .clamp(20, 100)
            .min(candidates.len());
        let mut used_across_mixes = HashSet::<usize>::new();

        for variant in 0..3 {
            let mut random_state = seed ^ (variant as u64 + 1).wrapping_mul(0xD6E8_FEB8_6659_FD93);
            // Precompute normalized artist/album keys once. The previous version
            // compared every candidate against every selected track on every
            // selection step (quadratic in the requested mix size). Counters make
            // those variety penalties constant-time lookups instead.
            let mut remaining = candidates
                .iter()
                .copied()
                .map(|index| {
                    let track = &self.tracks[index];
                    let rank = rank_by_track
                        .get(&index)
                        .copied()
                        .unwrap_or(ranked.len() + index);
                    let familiarity = 1.0 / (1.0 + rank as f32 * 0.045);
                    let unplayed = if played_rank.contains_key(&track.path) {
                        0.0
                    } else {
                        1.0
                    };
                    let liked = if self.liked_songs.contains(&track.path) {
                        1.0
                    } else {
                        0.0
                    };
                    let genre_key = track.genre.trim().to_lowercase();
                    let genre_count = genre_frequency.get(&genre_key).copied().unwrap_or(0);
                    let rarity = if genre_count == 0 {
                        0.0
                    } else {
                        1.0 / (genre_count as f32).sqrt()
                    };
                    let freshness = if newest_added == 0 {
                        0.0
                    } else {
                        track.added as f32 / newest_added as f32
                    };
                    let recent_penalty =
                        played_rank
                            .get(&track.path)
                            .copied()
                            .map_or(0.0, |rank| match rank {
                                0..=3 => 1.25,
                                4..=10 => 0.65,
                                11..=24 => 0.2,
                                _ => 0.0,
                            });
                    let jitter =
                        (next_random(&mut random_state) >> 40) as f32 / ((1_u64 << 24) - 1) as f32;
                    let score = match variant {
                        0 => {
                            familiarity * 5.2 + liked * 1.3 + unplayed * 0.85 - recent_penalty
                                + jitter * 0.9
                        }
                        1 => {
                            familiarity * 2.0 + unplayed * 2.7 + rarity * 3.0 + freshness * 0.8
                                - recent_penalty * 0.8
                                + jitter * 1.45
                        }
                        _ => {
                            familiarity * 0.8 + unplayed * 1.9 + rarity * 4.1 + freshness * 1.25
                                - recent_penalty * 0.45
                                + jitter * 3.0
                        }
                    };
                    let artist_key = track.artist.to_ascii_lowercase();
                    let album_key = if is_unknown(&track.album) {
                        String::new()
                    } else {
                        track.album.to_ascii_lowercase()
                    };
                    (index, score, artist_key, album_key)
                })
                .collect::<Vec<_>>();

            let mut selected: Vec<usize> = Vec::with_capacity(target_size);
            let mut selected_artist_counts = HashMap::<String, usize>::new();
            let mut selected_album_counts = HashMap::<String, usize>::new();
            while !remaining.is_empty() && selected.len() < target_size {
                let has_unused_tracks = remaining
                    .iter()
                    .any(|(index, _, _, _)| !used_across_mixes.contains(index));
                let best_position = remaining
                    .iter()
                    .enumerate()
                    .map(|(position, (index, base_score, artist_key, album_key))| {
                        let repeated_artist =
                            selected_artist_counts.get(artist_key).copied().unwrap_or(0) as f32;
                        let repeated_album =
                            selected_album_counts.get(album_key).copied().unwrap_or(0) as f32;
                        let used_penalty = if has_unused_tracks && used_across_mixes.contains(index)
                        {
                            1000.0
                        } else {
                            0.0
                        };
                        let variety_penalty = repeated_artist * self.recommendation_variety * 0.9
                            + repeated_album * self.recommendation_variety * 1.35;
                        (position, *base_score - used_penalty - variety_penalty)
                    })
                    .max_by(|left, right| {
                        left.1
                            .total_cmp(&right.1)
                            .then_with(|| right.0.cmp(&left.0))
                    })
                    .map(|(position, _)| position)
                    .expect("remaining mix candidates are non-empty");
                let (picked, _, artist_key, album_key) = remaining.swap_remove(best_position);
                selected.push(picked);
                *selected_artist_counts.entry(artist_key).or_default() += 1;
                if !album_key.is_empty() {
                    *selected_album_counts.entry(album_key).or_default() += 1;
                }
                used_across_mixes.insert(picked);
            }
            mixes[variant] = selected;
        }
        mixes
    }
}

fn properties_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.label(RichText::new(label).strong());
    ui.label(value);
    ui.end_row();
}

fn read_file_details(path: &Path) -> FileDetails {
    let mut details = FileDetails::default();
    match fs::metadata(path) {
        Ok(metadata) => {
            details.size_bytes = Some(metadata.len());
            details.modified = metadata
                .modified()
                .ok()
                .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_secs());
        }
        Err(error) => details.error = Some(format!("Could not read file: {error}")),
    }
    details.format = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_uppercase());
    match Probe::open(path).and_then(|probe| probe.read()) {
        Ok(tagged) => {
            let properties = tagged.properties();
            details.bitrate_kbps = properties
                .audio_bitrate()
                .or_else(|| properties.overall_bitrate())
                .filter(|&value| value > 0);
            details.sample_rate_hz = properties.sample_rate().filter(|&value| value > 0);
            details.channels = properties.channels().filter(|&value| value > 0);
            details.bit_depth = properties.bit_depth().filter(|&value| value > 0);
        }
        Err(error) => {
            if details.error.is_none() {
                details.error = Some(format!("Could not read audio properties: {error}"));
            }
        }
    }
    details
}

fn find_duplicate_groups(
    candidates: Vec<DuplicateCandidate>,
    check_exact: bool,
    check_recordings: bool,
    check_partial: bool,
    cancel: &AtomicBool,
) -> Vec<DuplicateGroup> {
    let mut groups = Vec::new();
    let mut exact_paths = HashSet::<PathBuf>::new();

    // The expensive byte comparison is limited to files with the same size and
    // matching first/last samples. All work happens on the scan worker thread.
    if check_exact {
        let mut by_size = HashMap::<u64, Vec<DuplicateCandidate>>::new();
        for candidate in &candidates {
            if cancel.load(Ordering::Acquire) {
                return Vec::new();
            }
            if let Ok(metadata) = fs::metadata(&candidate.path) {
                by_size
                    .entry(metadata.len())
                    .or_default()
                    .push(candidate.clone());
            }
        }

        for (size, same_size) in by_size {
            if cancel.load(Ordering::Acquire) {
                return Vec::new();
            }
            if same_size.len() < 2 {
                continue;
            }
            let mut by_sample = HashMap::<u64, Vec<DuplicateCandidate>>::new();
            for candidate in same_size {
                if let Some(fingerprint) = file_sample_fingerprint(&candidate.path, size) {
                    by_sample.entry(fingerprint).or_default().push(candidate);
                }
            }
            for possible_matches in by_sample.into_values() {
                if possible_matches.len() < 2 {
                    continue;
                }
                let mut verified_groups = Vec::<Vec<DuplicateTrack>>::new();
                for candidate in possible_matches {
                    if cancel.load(Ordering::Acquire) {
                        return Vec::new();
                    }
                    let mut duplicate_group = None;
                    for (group_index, group) in verified_groups.iter().enumerate() {
                        if files_are_identical(&group[0].path, &candidate.path, size, cancel)
                            .unwrap_or(false)
                        {
                            duplicate_group = Some(group_index);
                            break;
                        }
                    }
                    let track = DuplicateTrack {
                        path: candidate.path,
                        title: candidate.title,
                        artist: candidate.artist,
                        duration: candidate.duration,
                        size_bytes: size,
                    };
                    if let Some(group_index) = duplicate_group {
                        verified_groups[group_index].push(track);
                    } else {
                        verified_groups.push(vec![track]);
                    }
                }
                for mut duplicate_group in verified_groups {
                    if duplicate_group.len() < 2 {
                        continue;
                    }
                    duplicate_group.sort_by(|left, right| left.path.cmp(&right.path));
                    exact_paths.extend(duplicate_group.iter().map(|track| track.path.clone()));
                    groups.push(DuplicateGroup {
                        kind: DuplicateKind::BitForBit,
                        tracks: duplicate_group,
                    });
                }
            }
        }
    }

    if cancel.load(Ordering::Acquire) {
        return Vec::new();
    }

    let mut metadata_tracks = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        if cancel.load(Ordering::Acquire) {
            return Vec::new();
        }
        if exact_paths.contains(&candidate.path) {
            continue;
        }
        let size_bytes = fs::metadata(&candidate.path)
            .map(|metadata| metadata.len())
            .unwrap_or_default();
        metadata_tracks.push(DuplicateTrack {
            path: candidate.path,
            title: candidate.title,
            artist: candidate.artist,
            duration: candidate.duration,
            size_bytes,
        });
    }
    let mut matched_metadata_paths = HashSet::<PathBuf>::new();

    if check_recordings {
        let mut by_song = HashMap::<(String, String), Vec<DuplicateTrack>>::new();
        for track in &metadata_tracks {
            let title = normalize_duplicate_text(&track.title);
            let artist = if is_unknown(&track.artist) {
                String::new()
            } else {
                normalize_duplicate_text(&track.artist)
            };
            if !title.is_empty() && !artist.is_empty() && track.duration > 0.0 {
                by_song
                    .entry((title, artist))
                    .or_default()
                    .push(track.clone());
            }
        }
        for tracks in by_song.into_values() {
            if cancel.load(Ordering::Acquire) {
                return Vec::new();
            }
            for mut cluster in duration_clusters(tracks, 2.5) {
                if cluster.len() < 2 {
                    continue;
                }
                cluster.sort_by(|left, right| left.path.cmp(&right.path));
                matched_metadata_paths.extend(cluster.iter().map(|track| track.path.clone()));
                groups.push(DuplicateGroup {
                    kind: DuplicateKind::SameRecording,
                    tracks: cluster,
                });
            }
        }
    }

    if check_partial {
        let mut by_title = HashMap::<String, Vec<DuplicateTrack>>::new();
        for track in &metadata_tracks {
            let title = normalize_duplicate_text(&track.title);
            if !title.is_empty()
                && track.duration > 0.0
                && !matched_metadata_paths.contains(&track.path)
            {
                by_title.entry(title).or_default().push(track.clone());
            }
        }
        for tracks in by_title.into_values() {
            if cancel.load(Ordering::Acquire) {
                return Vec::new();
            }
            for mut cluster in duration_clusters(tracks, 6.0) {
                if cluster.len() < 2 {
                    continue;
                }
                let artists = cluster
                    .iter()
                    .map(|track| {
                        if is_unknown(&track.artist) {
                            String::new()
                        } else {
                            normalize_duplicate_text(&track.artist)
                        }
                    })
                    .collect::<HashSet<_>>();
                let runtime_spread = cluster
                    .iter()
                    .map(|track| track.duration)
                    .fold((f64::INFINITY, f64::NEG_INFINITY), |(min, max), value| {
                        (min.min(value), max.max(value))
                    });
                let has_uncertain_metadata = artists.len() > 1 || artists.contains("");
                let has_runtime_difference = runtime_spread.1 - runtime_spread.0 > 2.5;
                if !has_uncertain_metadata && !has_runtime_difference {
                    continue;
                }
                cluster.sort_by(|left, right| left.path.cmp(&right.path));
                groups.push(DuplicateGroup {
                    kind: DuplicateKind::PartialMatch,
                    tracks: cluster,
                });
            }
        }
    }

    groups.sort_by(|left, right| {
        let rank = |kind| match kind {
            DuplicateKind::BitForBit => 0,
            DuplicateKind::SameRecording => 1,
            DuplicateKind::PartialMatch => 2,
        };
        rank(left.kind).cmp(&rank(right.kind)).then_with(|| {
            left.tracks[0]
                .title
                .to_lowercase()
                .cmp(&right.tracks[0].title.to_lowercase())
        })
    });
    groups
}

fn duration_clusters(
    mut tracks: Vec<DuplicateTrack>,
    tolerance_seconds: f64,
) -> Vec<Vec<DuplicateTrack>> {
    tracks.retain(|track| track.duration.is_finite() && track.duration > 0.0);
    tracks.sort_by(|left, right| left.duration.total_cmp(&right.duration));
    let mut groups = Vec::new();
    let mut current = Vec::new();
    let mut anchor_duration: Option<f64> = None;
    for track in tracks {
        match anchor_duration {
            Some(anchor) if track.duration - anchor <= tolerance_seconds => {
                current.push(track);
            }
            Some(_) => {
                if !current.is_empty() {
                    groups.push(std::mem::take(&mut current));
                }
                anchor_duration = Some(track.duration);
                current.push(track);
            }
            None => {
                anchor_duration = Some(track.duration);
                current.push(track);
            }
        }
    }
    if !current.is_empty() {
        groups.push(current);
    }
    groups
}

fn normalize_duplicate_text(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn file_content_fingerprint(path: &Path) -> std::io::Result<String> {
    // Stable FNV-1a 128-bit digest. This reads every byte in bounded memory and
    // is used only as an exact-content identity key, never for approximate matches.
    const OFFSET_BASIS: u128 = 0x6c62272e07bb014262b821756295c58d;
    const FNV_PRIME: u128 = 0x0000000001000000000000000000013B;
    const BUFFER_SIZE: usize = 64 * 1024;

    let mut file = File::open(path)?;
    let mut buffer = [0_u8; BUFFER_SIZE];
    let mut fingerprint = OFFSET_BASIS;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        for byte in &buffer[..read] {
            fingerprint ^= u128::from(*byte);
            fingerprint = fingerprint.wrapping_mul(FNV_PRIME);
        }
    }
    Ok(format!("{fingerprint:032x}"))
}

fn file_sample_fingerprint(path: &Path, size: u64) -> Option<u64> {
    const SAMPLE_BYTES: usize = 16 * 1024;
    let mut file = File::open(path).ok()?;
    let mut hasher = DefaultHasher::new();
    size.hash(&mut hasher);

    let first_length = size.min(SAMPLE_BYTES as u64) as usize;
    let mut first = vec![0; first_length];
    file.read_exact(&mut first).ok()?;
    hasher.write(&first);

    if size > first_length as u64 {
        let last_length = size.min(SAMPLE_BYTES as u64) as usize;
        file.seek(SeekFrom::Start(size.saturating_sub(last_length as u64)))
            .ok()?;
        let mut last = vec![0; last_length];
        file.read_exact(&mut last).ok()?;
        hasher.write(&last);
    }
    Some(hasher.finish())
}

fn files_are_identical(
    left: &Path,
    right: &Path,
    size: u64,
    cancel: &AtomicBool,
) -> std::io::Result<bool> {
    const CHUNK_SIZE: usize = 64 * 1024;
    let mut left_file = File::open(left)?;
    let mut right_file = File::open(right)?;
    let mut left_buffer = vec![0; CHUNK_SIZE];
    let mut right_buffer = vec![0; CHUNK_SIZE];
    let mut remaining = size;
    while remaining > 0 {
        if cancel.load(Ordering::Acquire) {
            return Ok(false);
        }
        let chunk_length = remaining.min(CHUNK_SIZE as u64) as usize;
        left_file.read_exact(&mut left_buffer[..chunk_length])?;
        right_file.read_exact(&mut right_buffer[..chunk_length])?;
        if left_buffer[..chunk_length] != right_buffer[..chunk_length] {
            return Ok(false);
        }
        remaining -= chunk_length as u64;
    }
    Ok(true)
}

fn format_elapsed_since(timestamp: u64) -> String {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .saturating_sub(timestamp);
    match elapsed {
        0..=59 => "just now".into(),
        60..=3_599 => format!("{}m ago", elapsed / 60),
        3_600..=86_399 => format!("{}h ago", elapsed / 3_600),
        _ => format!("{}d ago", elapsed / 86_400),
    }
}

fn format_file_size(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    let bytes = bytes as f64;
    if bytes >= GIB {
        format!("{:.2} GB", bytes / GIB)
    } else if bytes >= MIB {
        format!("{:.1} MB", bytes / MIB)
    } else if bytes >= KIB {
        format!("{:.0} KB", bytes / KIB)
    } else {
        format!("{} B", bytes as u64)
    }
}

fn build_deleted_fingerprint_rules(records: &[DeletedSongRecord]) -> Vec<DeletedFingerprintRule> {
    let mut rules = HashMap::<(u64, String), HashSet<PathBuf>>::new();
    for record in records {
        let Some(fingerprint) = record.fingerprint.as_ref() else {
            continue;
        };
        rules
            .entry((record.size_bytes, fingerprint.clone()))
            .or_default()
            .extend(record.preserved_paths.iter().cloned());
    }
    rules
        .into_iter()
        .map(
            |((size_bytes, fingerprint), preserved_paths)| DeletedFingerprintRule {
                fingerprint,
                size_bytes,
                preserved_paths,
            },
        )
        .collect()
}

fn should_auto_remove_deleted_file(path: &Path, rules: &[DeletedFingerprintRule]) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    let size_bytes = metadata.len();
    let possible_rules = rules
        .iter()
        .filter(|rule| rule.size_bytes == size_bytes && !rule.preserved_paths.contains(path))
        .collect::<Vec<_>>();
    if possible_rules.is_empty() {
        return false;
    }
    let Ok(fingerprint) = file_content_fingerprint(path) else {
        return false;
    };
    possible_rules
        .iter()
        .any(|rule| rule.fingerprint == fingerprint)
}

fn scan_folders(
    folders: &[PathBuf],
    deleted_rules: &[DeletedFingerprintRule],
    auto_purge_deleted: bool,
) -> ScanResult {
    scan_folders_with_settings(
        folders,
        deleted_rules,
        auto_purge_deleted,
        0,
        Arc::new(AtomicUsize::new(0)),
    )
}

fn scan_folders_with_settings(
    folders: &[PathBuf],
    deleted_rules: &[DeletedFingerprintRule],
    auto_purge_deleted: bool,
    scan_rate_per_second: usize,
    scan_progress: Arc<AtomicUsize>,
) -> ScanResult {
    let mut errors = Vec::new();
    let mut auto_deleted_count = 0;
    let mut auto_delete_failures = 0;
    let mut auto_deleted_paths = Vec::new();
    let mut audio_paths: Vec<PathBuf> = Vec::new();
    let mut pending = folders.to_vec();

    // Phase 1: walk the folders (cheap) and apply the deleted-song protection.
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
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(error) => {
                    errors.push(format!("{}: {error}", path.display()));
                    continue;
                }
            };
            if file_type.is_dir() {
                pending.push(path);
            } else if file_type.is_file() && is_audio_file(&path) {
                if auto_purge_deleted && should_auto_remove_deleted_file(&path, deleted_rules) {
                    match fs::remove_file(&path) {
                        Ok(()) => {
                            auto_deleted_count += 1;
                            auto_deleted_paths.push(path.clone());
                            continue;
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                        Err(error) => {
                            auto_delete_failures += 1;
                            errors.push(format!(
                                "Could not auto-remove previously deleted file {}: {error}",
                                path.display()
                            ));
                        }
                    }
                }
                audio_paths.push(path);
            }
        }
    }

    // Phase 2: read tags on several worker threads. This is the slow part of a
    // scan (disk I/O plus tag parsing), so it is spread across cores. The rate
    // limit (songs per second, 0 = unlimited) is shared by all workers.
    let worker_count = thread::available_parallelism()
        .map_or(4, |count| count.get())
        .clamp(2, 8)
        .min(audio_paths.len().max(1));
    let next_path = AtomicUsize::new(0);
    let pacer = Mutex::new((Instant::now(), 0usize));
    let collected: Mutex<Vec<Track>> = Mutex::new(Vec::with_capacity(audio_paths.len()));
    thread::scope(|scope| {
        for _ in 0..worker_count {
            scope.spawn(|| {
                let mut artwork_cache = HashMap::<u64, Vec<Arc<Vec<u8>>>>::new();
                let mut local = Vec::new();
                loop {
                    let position = next_path.fetch_add(1, Ordering::Relaxed);
                    let Some(path) = audio_paths.get(position) else {
                        break;
                    };
                    if scan_rate_per_second > 0 {
                        let mut window = pacer.lock().unwrap_or_else(|error| error.into_inner());
                        if window.1 >= scan_rate_per_second {
                            let elapsed = window.0.elapsed();
                            if elapsed < Duration::from_secs(1) {
                                thread::sleep(Duration::from_secs(1) - elapsed);
                            }
                            *window = (Instant::now(), 0);
                        }
                        window.1 += 1;
                    }
                    scan_progress.fetch_add(1, Ordering::Relaxed);
                    local.push(read_track(path.clone(), &mut artwork_cache));
                }
                collected
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .extend(local);
            });
        }
    });
    let mut tracks = collected
        .into_inner()
        .unwrap_or_else(|error| error.into_inner());

    // Each worker had its own artwork cache; share identical cover images again
    // so large libraries do not keep duplicate copies in memory.
    let mut shared_artwork = HashMap::<u64, Vec<Arc<Vec<u8>>>>::new();
    for track in &mut tracks {
        if let Some(artwork) = track.artwork.take() {
            let mut hasher = DefaultHasher::new();
            artwork.as_slice().hash(&mut hasher);
            let bucket = shared_artwork.entry(hasher.finish()).or_default();
            if let Some(existing) = bucket
                .iter()
                .find(|candidate| candidate.as_slice() == artwork.as_slice())
            {
                track.artwork = Some(Arc::clone(existing));
            } else {
                bucket.push(Arc::clone(&artwork));
                track.artwork = Some(artwork);
            }
        }
    }

    // Cache lowercase sort keys once instead of allocating them repeatedly in
    // the O(n log n) comparator for large libraries. The path is the final
    // tie-breaker so the order does not depend on worker timing.
    tracks.sort_by_cached_key(|track| {
        (
            track.artist.to_lowercase(),
            track.album.to_lowercase(),
            track.title.to_lowercase(),
            track.path.clone(),
        )
    });
    ScanResult {
        tracks,
        error: if errors.is_empty() {
            None
        } else {
            Some(format!(
                "Some folders or files could not be processed: {}",
                errors.join("; ")
            ))
        },
        auto_deleted_count,
        auto_delete_failures,
        auto_deleted_paths,
    }
}

fn read_track(path: PathBuf, artwork_cache: &mut HashMap<u64, Vec<Arc<Vec<u8>>>>) -> Track {
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
            track.artwork = tag.pictures().first().and_then(|picture| {
                let data = picture.data();
                if data.is_empty() || data.len() > MAX_CACHED_ARTWORK_BYTES {
                    return None;
                }
                let mut hasher = DefaultHasher::new();
                data.hash(&mut hasher);
                let bucket = artwork_cache.entry(hasher.finish()).or_default();
                if let Some(shared) = bucket.iter().find(|candidate| candidate.as_slice() == data) {
                    return Some(Arc::clone(shared));
                }
                let shared = Arc::new(data.to_vec());
                bucket.push(Arc::clone(&shared));
                Some(shared)
            });
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

fn next_random(state: &mut u64) -> u64 {
    if *state == 0 {
        *state = 0x9E37_79B9_7F4A_7C15;
    }
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
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
        items.swap(index, (next_random(&mut state) as usize) % (index + 1));
    }
}

fn animated_nav_item(
    ui: &mut egui::Ui,
    id: &str,
    label: &str,
    icon: Option<&TextureHandle>,
    selected: bool,
    accent: Color32,
) -> bool {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 36.0), egui::Sense::click());
    let selected_t =
        ui.ctx()
            .animate_bool_with_time(egui::Id::new(("nav-selected", id)), selected, 0.20);
    let hover_t =
        ui.ctx()
            .animate_bool_with_time(egui::Id::new(("nav-hover", id)), response.hovered(), 0.14);
    let idle = Color32::from_rgb(12, 12, 16);
    let hover = Color32::from_rgb(27, 29, 43);
    let active = Color32::from_rgb(35, 39, 64);
    let base = mix_color(idle, hover, hover_t);
    let fill = mix_color(base, active, selected_t);
    ui.painter()
        .rect_filled(rect, egui::CornerRadius::same(9), fill);
    if let Some(icon) = icon {
        let icon_rect = egui::Rect::from_center_size(
            egui::pos2(rect.left() + 27.0, rect.center().y),
            Vec2::splat(18.0),
        );
        ui.painter().image(
            icon.id(),
            icon_rect,
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }
    if selected_t > 0.01 || hover_t > 0.01 {
        let stripe = egui::Rect::from_min_size(
            rect.min + Vec2::new(0.0, 9.0),
            Vec2::new(3.0 + selected_t * 1.5, rect.height() - 18.0),
        );
        ui.painter().rect_filled(
            stripe,
            egui::CornerRadius::same(2),
            accent.linear_multiply(selected_t.max(hover_t * 0.45)),
        );
    }
    let text_color = mix_color(
        Color32::from_rgb(165, 166, 178),
        Color32::WHITE,
        selected_t.max(hover_t * 0.35),
    );
    ui.painter().text(
        egui::pos2(rect.left() + 44.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(14.0),
        text_color,
    );
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

fn mix_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let channel =
        |left: u8, right: u8| (left as f32 + (right as f32 - left as f32) * t).round() as u8;
    Color32::from_rgb(
        channel(a.r(), b.r()),
        channel(a.g(), b.g()),
        channel(a.b(), b.b()),
    )
}

fn main() -> eframe::Result<()> {
    let app_icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/app_icon.png"))
        .unwrap_or_default();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(APP_NAME)
            .with_icon(app_icon)
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
    fn file_details_do_not_invent_data_for_missing_files() {
        let details = read_file_details(Path::new("definitely-missing-file.mp3"));
        assert!(details.size_bytes.is_none());
        assert!(details.bitrate_kbps.is_none());
        assert!(details.sample_rate_hz.is_none());
        assert!(details.error.is_some());
    }

    #[test]
    fn old_scan_rate_setting_is_ignored_and_new_default_applies() {
        let decoded: Preferences =
            serde_json::from_str(r#"{"folders":[],"scan_rate_per_10_seconds":1000}"#)
                .expect("old settings parse");
        assert_eq!(decoded.scan_rate_per_second, default_scan_rate_per_second());
        assert!(decoded.daily_mixes.is_empty());
    }

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
        assert!(preferences.duplicate_scan_enabled);
        assert!(preferences.duplicate_check_exact);
        assert!(preferences.duplicate_check_recordings);
        assert!(!preferences.duplicate_check_partial);
        assert!(preferences.deleted_songs.is_empty());
        assert!(preferences.auto_purge_deleted);
        assert_eq!(preferences.scan_rate_per_second, 5_000);
    }

    #[test]
    fn scan_rate_round_trips_and_zero_means_unlimited() {
        let preferences = Preferences {
            scan_rate_per_second: 0,
            ..Preferences::default()
        };
        let encoded = serde_json::to_string(&preferences).expect("settings serialize");
        let decoded: Preferences = serde_json::from_str(&encoded).expect("settings deserialize");
        assert_eq!(decoded.scan_rate_per_second, 0);
        assert_eq!(Preferences::default().scan_rate_per_second, 5_000);
    }

    #[test]
    fn deleted_song_rules_round_trip_in_settings() {
        let record = DeletedSongRecord {
            fingerprint: Some("0123456789abcdef0123456789abcdef".into()),
            size_bytes: 4096,
            title: "Deleted track".into(),
            artist: "Test artist".into(),
            album: "Test album".into(),
            original_path: PathBuf::from("C:/Music/deleted.mp3"),
            deleted_at: 1_700_000_000,
            reason: "Manually deleted".into(),
            preserved_paths: vec![PathBuf::from("C:/Music/kept-copy.mp3")],
        };
        let preferences = Preferences {
            deleted_songs: vec![record],
            auto_purge_deleted: false,
            ..Preferences::default()
        };
        let encoded = serde_json::to_string(&preferences).expect("settings serialize");
        let decoded: Preferences = serde_json::from_str(&encoded).expect("settings deserialize");
        assert_eq!(decoded.deleted_songs.len(), 1);
        assert_eq!(decoded.deleted_songs[0].title, "Deleted track");
        assert_eq!(decoded.deleted_songs[0].preserved_paths.len(), 1);
        assert!(!decoded.auto_purge_deleted);
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
    fn daily_mix_generator_builds_three_varied_unique_track_sets() {
        let mut app = PlayerApp::new();
        app.recommendation_size = 20;
        app.tracks = (0..90)
            .map(|index| Track {
                path: PathBuf::from(format!("track-{index}.mp3")),
                title: format!("Track {index}"),
                artist: format!("Artist {index}"),
                album: format!("Album {index}"),
                genre: ["Ambient", "Electronic", "Jazz"][index % 3].into(),
                added: index as u64 + 1,
                ..Track::default()
            })
            .collect();

        let mixes = app.build_daily_mixes(0x1234_5678_9ABC_DEF0);
        assert_eq!(mixes, app.build_daily_mixes(0x1234_5678_9ABC_DEF0));
        assert!(mixes.iter().all(|mix| mix.len() == 20));
        assert!(mixes.iter().all(|mix| {
            let unique = mix.iter().copied().collect::<HashSet<_>>();
            unique.len() == mix.len()
        }));
        let combined = mixes.iter().flatten().copied().collect::<HashSet<_>>();
        assert_eq!(combined.len(), 60);
    }

    #[test]
    fn content_fingerprint_matches_only_identical_complete_files() {
        let directory = duplicate_test_directory();
        let first = directory.join("first.mp3");
        let second = directory.join("second.mp3");
        let changed = directory.join("changed.mp3");
        fs::write(&first, b"identical full song bytes").expect("write first file");
        fs::write(&second, b"identical full song bytes").expect("write second file");
        fs::write(&changed, b"identical full song byteS").expect("write changed file");

        let first_fingerprint = file_content_fingerprint(&first).expect("fingerprint first");
        assert_eq!(
            first_fingerprint,
            file_content_fingerprint(&second).expect("fingerprint second")
        );
        assert_ne!(
            first_fingerprint,
            file_content_fingerprint(&changed).expect("fingerprint changed")
        );
        fs::remove_dir_all(directory).expect("remove test directory");
    }

    #[test]
    fn scan_quietly_removes_a_reappearing_deleted_file_by_exact_content() {
        let directory = duplicate_test_directory();
        let reappeared = directory.join("reappeared.mp3");
        fs::write(&reappeared, b"the exact deleted song").expect("write reappeared file");
        let size_bytes = fs::metadata(&reappeared).expect("metadata").len();
        let fingerprint = file_content_fingerprint(&reappeared).expect("fingerprint");
        let rule = DeletedFingerprintRule {
            fingerprint,
            size_bytes,
            preserved_paths: HashSet::new(),
        };

        let result = scan_folders(&[directory.clone()], &[rule], true);
        assert_eq!(result.auto_deleted_count, 1);
        assert!(result.tracks.is_empty());
        assert!(!reappeared.exists());
        fs::remove_dir_all(directory).expect("remove test directory");
    }

    #[test]
    fn scan_keeps_the_explicitly_preserved_copy_of_a_deleted_song() {
        let directory = duplicate_test_directory();
        let kept = directory.join("kept.mp3");
        let reappeared = directory.join("reappeared.mp3");
        fs::write(&kept, b"same exact song bytes").expect("write kept file");
        fs::write(&reappeared, b"same exact song bytes").expect("write reappeared file");
        let size_bytes = fs::metadata(&kept).expect("metadata").len();
        let fingerprint = file_content_fingerprint(&kept).expect("fingerprint");
        let rule = DeletedFingerprintRule {
            fingerprint,
            size_bytes,
            preserved_paths: HashSet::from([kept.clone()]),
        };

        let result = scan_folders(&[directory.clone()], &[rule], true);
        assert_eq!(result.auto_deleted_count, 1);
        assert!(kept.exists());
        assert!(!reappeared.exists());
        assert_eq!(result.tracks.len(), 1);
        fs::remove_dir_all(directory).expect("remove test directory");
    }

    #[test]
    fn scan_does_not_auto_remove_when_protection_is_disabled() {
        let directory = duplicate_test_directory();
        let reappeared = directory.join("reappeared.mp3");
        fs::write(&reappeared, b"the exact deleted song").expect("write reappeared file");
        let size_bytes = fs::metadata(&reappeared).expect("metadata").len();
        let fingerprint = file_content_fingerprint(&reappeared).expect("fingerprint");
        let rule = DeletedFingerprintRule {
            fingerprint,
            size_bytes,
            preserved_paths: HashSet::new(),
        };

        let result = scan_folders(&[directory.clone()], &[rule], false);
        assert_eq!(result.auto_deleted_count, 0);
        assert!(reappeared.exists());
        assert_eq!(result.tracks.len(), 1);
        fs::remove_dir_all(directory).expect("remove test directory");
    }

    fn duplicate_test_candidate(
        path: &Path,
        title: &str,
        artist: &str,
        duration: f64,
    ) -> DuplicateCandidate {
        DuplicateCandidate {
            path: path.to_path_buf(),
            title: title.into(),
            artist: artist.into(),
            duration,
        }
    }

    fn duplicate_test_directory() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "lilt-duplicate-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        fs::create_dir_all(&path).expect("create duplicate test directory");
        path
    }

    #[test]
    fn duplicate_detector_finds_bit_for_bit_copies_and_groups_them_separately() {
        let directory = duplicate_test_directory();
        let first = directory.join("first.mp3");
        let second = directory.join("second.mp3");
        let separate = directory.join("different.mp3");
        fs::write(&first, b"the exact same audio bytes").expect("write first file");
        fs::write(&second, b"the exact same audio bytes").expect("write second file");
        fs::write(&separate, b"a different file with another size").expect("write third file");

        let groups = find_duplicate_groups(
            vec![
                duplicate_test_candidate(&first, "Song", "Artist", 120.0),
                duplicate_test_candidate(&second, "Song", "Artist", 120.0),
                duplicate_test_candidate(&separate, "Other song", "Other artist", 130.0),
            ],
            true,
            true,
            true,
            &AtomicBool::new(false),
        );

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].kind, DuplicateKind::BitForBit);
        assert_eq!(groups[0].tracks.len(), 2);
        fs::remove_dir_all(directory).expect("remove duplicate test directory");
    }

    #[test]
    fn duplicate_detector_verifies_full_bytes_after_matching_file_samples() {
        let directory = duplicate_test_directory();
        let first = directory.join("first.mp3");
        let second = directory.join("second.mp3");
        let prefix = vec![7_u8; 20 * 1024];
        let suffix = vec![9_u8; 20 * 1024];
        let mut first_bytes = prefix.clone();
        first_bytes.extend(vec![1_u8; 8 * 1024]);
        first_bytes.extend_from_slice(&suffix);
        let mut second_bytes = prefix;
        second_bytes.extend(vec![2_u8; 8 * 1024]);
        second_bytes.extend_from_slice(&suffix);
        fs::write(&first, first_bytes).expect("write first file");
        fs::write(&second, second_bytes).expect("write second file");

        let groups = find_duplicate_groups(
            vec![
                duplicate_test_candidate(&first, "Song A", "Artist A", 100.0),
                duplicate_test_candidate(&second, "Song B", "Artist B", 110.0),
            ],
            true,
            false,
            false,
            &AtomicBool::new(false),
        );

        assert!(groups.is_empty());
        fs::remove_dir_all(directory).expect("remove duplicate test directory");
    }

    #[test]
    fn duplicate_detector_finds_same_recording_in_different_encodes() {
        let directory = duplicate_test_directory();
        let first = directory.join("song.mp3");
        let second = directory.join("song.flac");
        fs::write(&first, b"mp3 encode data").expect("write mp3");
        fs::write(&second, b"flac encode with different bytes").expect("write flac");

        let groups = find_duplicate_groups(
            vec![
                duplicate_test_candidate(&first, "Blue Sky!", "The Artist", 182.0),
                duplicate_test_candidate(&second, "blue sky", "the artist", 183.0),
            ],
            true,
            true,
            true,
            &AtomicBool::new(false),
        );

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].kind, DuplicateKind::SameRecording);
        assert_eq!(groups[0].tracks.len(), 2);
        fs::remove_dir_all(directory).expect("remove duplicate test directory");
    }

    #[test]
    fn duplicate_detector_keeps_partial_matches_separate_from_exact_and_version_groups() {
        let directory = duplicate_test_directory();
        let first = directory.join("artist-one.mp3");
        let second = directory.join("artist-two.mp3");
        fs::write(&first, b"recording one").expect("write first variant");
        fs::write(&second, b"recording two differs").expect("write second variant");

        let groups = find_duplicate_groups(
            vec![
                duplicate_test_candidate(&first, "Midnight Road", "Artist One", 201.0),
                duplicate_test_candidate(&second, "midnight road", "Artist Two", 202.0),
            ],
            true,
            true,
            true,
            &AtomicBool::new(false),
        );

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].kind, DuplicateKind::PartialMatch);
        fs::remove_dir_all(directory).expect("remove duplicate test directory");
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

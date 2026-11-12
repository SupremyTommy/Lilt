
// Lilt source — refreshed attachment revision
use std::process::Command;
use std::{
    collections::{hash_map::DefaultHasher, BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    fs::{self, File},
    hash::{Hash, Hasher},
    io::{BufReader, Read, Seek, SeekFrom, Write},
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
use opus_pure::{OggOpusReader, OggPacket, OpusDecoder, Trim, MAX_PACKET_SAMPLES};
use rodio::{Decoder, OutputStream, Sink, Source};
use serde::{Deserialize, Serialize};
use souvlaki::{MediaControlEvent, MediaControls, PlatformConfig};

const APP_NAME: &str = "Lilt";
// Large embedded cover images are skipped to prevent libraries with huge art
// from retaining gigabytes of duplicated image data in RAM.
const MAX_CACHED_ARTWORK_BYTES: usize = 2 * 1024 * 1024;
const AUDIO_EXTENSIONS: &[&str] = &["mp3", "flac", "wav", "m4a", "ogg", "opus", "aac"];

/// Streaming Ogg Opus decoder. Decodes one packet at a time so long tracks do
/// not need to be expanded into a multi-gigabyte PCM buffer before playback.
struct OpusFileSource {
    reader: OggOpusReader<BufReader<File>>,
    decoder: OpusDecoder,
    trim: Trim,
    packet: OggPacket,
    decoded: Vec<f32>,
    pending: std::ops::Range<usize>,
    channels: u16,
    sample_rate: u32,
    ended: bool,
}

#[derive(Clone, Debug)]
struct UpdateInfo {
    version: String,
    download_url: String,
}

enum UpdateMessage {
    Check(Result<Option<UpdateInfo>, String>),
    Download(Result<(), String>),
}

impl OpusFileSource {
    fn open(file: File) -> Result<Self, String> {
        let reader = OggOpusReader::new(BufReader::new(file))
            .map_err(|error| format!("Invalid Ogg Opus stream: {error}"))?;
        let head = reader.head().clone();
        let channels = head.channel_count as usize;
        let decoder = head
            .decoder(48_000)
            .map_err(|error| format!("Could not initialize Opus decoder: {error}"))?;
        let trim = Trim::new(&head, 48_000, channels)
            .map_err(|error| format!("Could not prepare Opus stream: {error}"))?;
        Ok(Self {
            reader,
            decoder,
            trim,
            packet: OggPacket::default(),
            decoded: vec![0.0; MAX_PACKET_SAMPLES * channels],
            pending: 0..0,
            channels: channels as u16,
            sample_rate: 48_000,
            ended: false,
        })
    }
}

impl Iterator for OpusFileSource {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if !self.pending.is_empty() {
                let sample = self.decoded[self.pending.start];
                self.pending.start += 1;
                return Some(sample);
            }
            if self.ended {
                return None;
            }
            let has_packet = match self.reader.read_packet_into(&mut self.packet) {
                Ok(has_packet) => has_packet,
                Err(_) => {
                    self.ended = true;
                    return None;
                }
            };
            if !has_packet {
                self.ended = true;
                return None;
            }
            let frames =
                match self
                    .decoder
                    .decode(&self.packet.data, MAX_PACKET_SAMPLES, &mut self.decoded)
                {
                    Ok(frames) => frames,
                    Err(_) => {
                        self.ended = true;
                        return None;
                    }
                };
            self.pending = self
                .trim
                .keep_range(&self.packet, frames * self.channels as usize);
        }
    }
}

impl Source for OpusFileSource {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> u16 {
        self.channels
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

fn open_audio_source(path: &Path) -> Result<Box<dyn Source<Item = f32> + Send>, String> {
    let mut file = File::open(path).map_err(|error| error.to_string())?;
    let extension_is_opus = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("opus"));
    let mut probe = [0u8; 512];
    let bytes_read = file.read(&mut probe).unwrap_or(0);
    let contains_opus_head = probe[..bytes_read]
        .windows(b"OpusHead".len())
        .any(|window| window == b"OpusHead");
    file.seek(SeekFrom::Start(0))
        .map_err(|error| error.to_string())?;
    let is_opus = extension_is_opus || contains_opus_head;
    if is_opus {
        return OpusFileSource::open(file)
            .map(|source| Box::new(source) as Box<dyn Source<Item = f32> + Send>);
    }
    Decoder::new(BufReader::new(file))
        .map(|source| {
            Box::new(source.convert_samples::<f32>()) as Box<dyn Source<Item = f32> + Send>
        })
        .map_err(|error| error.to_string())
}

fn show_custom_window_controls(ui: &mut egui::Ui, context: &egui::Context) {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if ui
            .add_sized([42.0, 34.0], egui::Button::new("×"))
            .on_hover_text("Close")
            .clicked()
        {
            context.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        let maximized = context.input(|input| input.viewport().maximized.unwrap_or(false));
        if add_window_state_button(ui, maximized)
            .on_hover_text(if maximized { "Restore" } else { "Maximize" })
            .clicked()
        {
            context.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
        }
        if ui
            .add_sized([42.0, 34.0], egui::Button::new("—"))
            .on_hover_text("Minimize")
            .clicked()
        {
            context.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }
    });
}

fn add_window_state_button(ui: &mut egui::Ui, maximized: bool) -> egui::Response {
    let response = ui.add_sized([42.0, 34.0], egui::Button::new(""));
    let center = response.rect.center();
    let stroke = egui::Stroke::new(1.5_f32, Color32::from_rgb(220, 222, 230));
    let painter = ui.painter();
    let main_rect = egui::Rect::from_center_size(center, Vec2::splat(13.0));
    if maximized {
        let back_rect = main_rect.translate(Vec2::new(-2.5, 2.5));
        let front_rect = main_rect.translate(Vec2::new(2.5, -2.5));
        painter.rect_stroke(
            back_rect,
            egui::CornerRadius::ZERO,
            stroke,
            egui::StrokeKind::Inside,
        );
        painter.rect_stroke(
            front_rect,
            egui::CornerRadius::ZERO,
            stroke,
            egui::StrokeKind::Inside,
        );
    } else {
        painter.rect_stroke(
            main_rect,
            egui::CornerRadius::ZERO,
            stroke,
            egui::StrokeKind::Inside,
        );
    }
    response
}

fn icon_text_button(
    ui: &mut egui::Ui,
    icon: Option<&egui::TextureHandle>,
    label: impl Into<egui::WidgetText>,
    selected: bool,
) -> egui::Response {
    let label = label.into();
    let button = if let Some(icon) = icon {
        egui::Button::image_and_text(
            egui::Image::new(icon).fit_to_exact_size(Vec2::splat(18.0)),
            label,
        )
    } else {
        egui::Button::new(label)
    };
    ui.add(button.selected(selected))
}

fn show_custom_window_resize_grips(context: &egui::Context) {
    let (maximized, pressed, pointer_pos) = context.input(|input| {
        (
            input.viewport().maximized.unwrap_or(false),
            input.pointer.button_pressed(egui::PointerButton::Primary),
            input.pointer.interact_pos(),
        )
    });
    if maximized || !pressed {
        return;
    }
    let bounds = context.screen_rect();
    let Some(pointer_pos) = pointer_pos else {
        return;
    };
    let border = 8.0;
    let near_left = pointer_pos.x <= bounds.left() + border;
    let near_right = pointer_pos.x >= bounds.right() - border;
    let near_top = pointer_pos.y <= bounds.top() + border;
    let near_bottom = pointer_pos.y >= bounds.bottom() - border;
    let direction = match (near_left, near_right, near_top, near_bottom) {
        (true, _, true, _) => Some(egui::ResizeDirection::NorthWest),
        (_, true, true, _) => Some(egui::ResizeDirection::NorthEast),
        (true, _, _, true) => Some(egui::ResizeDirection::SouthWest),
        (_, true, _, true) => Some(egui::ResizeDirection::SouthEast),
        (_, _, true, _) => Some(egui::ResizeDirection::North),
        (_, _, _, true) => Some(egui::ResizeDirection::South),
        (true, _, _, _) => Some(egui::ResizeDirection::West),
        (_, true, _, _) => Some(egui::ResizeDirection::East),
        _ => None,
    };
    if let Some(direction) = direction {
        context.send_viewport_cmd(egui::ViewportCommand::BeginResize(direction));
    }
}

fn splash_background(elapsed: f32) -> Color32 {
    let progress = (elapsed / 0.25).clamp(0.0, 1.0);
    let eased = progress * progress * (3.0 - 2.0 * progress);
    Color32::from_rgb(
        (37.0 * (1.0 - eased)).round() as u8,
        (46.0 * (1.0 - eased)).round() as u8,
        (59.0 * (1.0 - eased)).round() as u8,
    )
}

fn show_splash_overlay(context: &egui::Context, elapsed: f32) {
    let fade_duration = 0.35;
    if !(0.5..(0.5 + fade_duration)).contains(&elapsed) {
        return;
    }
    let opacity = (1.0 - (elapsed - 0.5) / fade_duration).clamp(0.0, 1.0);
    let alpha = (opacity * 255.0).round() as u8;
    let bounds = context.screen_rect();
    egui::Area::new(egui::Id::new("splash-fade-overlay"))
        .order(egui::Order::Foreground)
        .fixed_pos(bounds.min)
        .show(context, |ui| {
            let (rect, _) = ui.allocate_exact_size(bounds.size(), egui::Sense::hover());
            ui.painter()
                .rect_filled(rect, 0.0, Color32::from_rgba_unmultiplied(0, 0, 0, alpha));
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "SupremyTommy",
                egui::FontId::proportional(34.0),
                Color32::from_rgba_unmultiplied(255, 255, 255, alpha),
            );
        });
    context.request_repaint_after(Duration::from_millis(16));
}

#[cfg(target_os = "windows")]
fn reveal_path_in_explorer(path: &Path) {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{
        System::Com::{CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_APARTMENTTHREADED},
        UI::Shell::{SHOpenFolderAndSelectItems, SHParseDisplayName},
    };

    let path = path.to_path_buf();
    let _ = thread::Builder::new()
        .name("reveal-in-explorer".into())
        .spawn(move || {
            let path = path.canonicalize().unwrap_or(path);
            if !path.is_file() {
                return;
            }
            let wide_path = path
                .as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect::<Vec<_>>();
            unsafe {
                if CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) < 0 {
                    return;
                }
                let mut item_id_list = std::ptr::null_mut();
                let parsed = SHParseDisplayName(
                    wide_path.as_ptr(),
                    std::ptr::null_mut(),
                    &mut item_id_list,
                    0,
                    std::ptr::null_mut(),
                );
                if parsed >= 0 && !item_id_list.is_null() {
                    // With no child array, the fully qualified file PIDL is selected
                    // in its parent folder by Explorer.
                    let _ = SHOpenFolderAndSelectItems(item_id_list, 0, std::ptr::null(), 0);
                    CoTaskMemFree(item_id_list.cast());
                } else if let Some(folder) = path.parent() {
                    // If the shell cannot parse the file path, at least open its folder.
                    let _ = std::process::Command::new("explorer.exe")
                        .arg(folder)
                        .spawn();
                }
                CoUninitialize();
            }
        });
}

const ACCENTS: [(&str, Color32); 7] = [
    ("Blue", Color32::from_rgb(54, 97, 255)),
    ("Sky", Color32::from_rgb(0, 162, 255)),
    ("Ice", Color32::from_rgb(204, 205, 211)),
    ("Violet", Color32::from_rgb(155, 103, 255)),
    ("Emerald", Color32::from_rgb(42, 190, 130)),
    ("Rose", Color32::from_rgb(238, 91, 135)),
    ("Amber", Color32::from_rgb(235, 165, 55)),
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
    #[serde(default)]
    replay_gain_track_db: Option<f32>,
    #[serde(default)]
    replay_gain_album_db: Option<f32>,
    #[serde(skip)]
    artwork: Option<Arc<Vec<u8>>>,
    #[serde(skip)]
    search_text: String,
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct Playlist {
    name: String,
    tracks: Vec<PathBuf>,
    #[serde(default)]
    rule: Option<SmartPlaylistRule>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
enum SmartPlaylistRule {
    #[default]
    LikedSongs,
    FollowedArtists,
    RecentlyAdded,
    Rediscover,
    FavoriteGenres,
}

impl SmartPlaylistRule {
    fn label(self) -> &'static str {
        match self {
            Self::LikedSongs => "Liked songs",
            Self::FollowedArtists => "Followed artists",
            Self::RecentlyAdded => "Recently added",
            Self::Rediscover => "Rediscover",
            Self::FavoriteGenres => "Favorite genres",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct TasteProfileBackup {
    version: u32,
    favorite_genres: Vec<String>,
    liked_artists: Vec<String>,
    liked_albums: Vec<String>,
    discovery: f32,
    prefer_unplayed: bool,
    recommendation_variety: f32,
    recommendation_size: usize,
    use_listening_history: bool,
}

#[derive(Clone, Copy)]
enum PlaylistExportFormat {
    Folder,
    Zip,
    M3u8,
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
enum SmartQueueMode {
    #[default]
    Off,
    WhenQueueEnds,
    KeepThreeReady,
}

impl SmartQueueMode {
    fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::WhenQueueEnds => "When queue ends",
            Self::KeepThreeReady => "Keep 3 recommendations ready",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
enum ReplayGainMode {
    #[default]
    Off,
    Track,
    Album,
}

impl ReplayGainMode {
    fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Track => "Track gain",
            Self::Album => "Album gain",
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum NowPlayingTab {
    #[default]
    UpNext,
    ForYou,
    Lyrics,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
enum GeminiModel {
    #[default]
    FlashLite,
    Flash,
}

impl GeminiModel {
    fn id(self) -> &'static str {
        match self {
            Self::FlashLite => "gemini-3.5-flash-lite",
            Self::Flash => "gemini-3.5-flash",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::FlashLite => "Gemini 3.5 Flash-Lite · lowest usage",
            Self::Flash => "Gemini 3.5 Flash · more capable",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
enum GeminiThinking {
    #[default]
    Minimal,
    Low,
}

impl GeminiThinking {
    fn id(self) -> &'static str {
        match self {
            Self::Minimal => "minimal",
            Self::Low => "low",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
struct AiUsage {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    thinking_tokens: u64,
    #[serde(default)]
    total_tokens: u64,
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
    #[serde(default = "legacy_welcome_completed")]
    welcome_completed: bool,
    #[serde(default)]
    folders: Vec<PathBuf>,
    #[serde(default)]
    favorite_genres: HashSet<String>,
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
    track_engagement: HashMap<PathBuf, TrackEngagement>,
    #[serde(default)]
    recommendation_feedback: HashMap<PathBuf, i8>,
    #[serde(default)]
    search_history: Vec<String>,
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
    #[serde(default)]
    smart_queue_mode: SmartQueueMode,
    #[serde(default = "default_true")]
    use_listening_history: bool,
    #[serde(default)]
    track_fade_seconds: f32,
    #[serde(default)]
    replay_gain_mode: ReplayGainMode,
    #[serde(default = "default_discovery")]
    discovery: f32,
    #[serde(default = "default_prefer_unplayed")]
    prefer_unplayed: bool,
    #[serde(default = "default_recommendation_variety")]
    recommendation_variety: f32,
    #[serde(default = "default_recommendation_size")]
    recommendation_size: usize,
    #[serde(default)]
    ai_recommendations_enabled: bool,
    #[serde(default)]
    google_ai_model: GeminiModel,
    #[serde(default)]
    google_ai_thinking: GeminiThinking,
    #[serde(default = "default_ai_mix_size")]
    ai_mix_size: usize,
    #[serde(default = "default_true")]
    ai_mix_include_liked: bool,
    #[serde(default = "default_true")]
    ai_mix_include_played: bool,
    #[serde(default = "default_true")]
    ai_mix_include_unplayed: bool,
    #[serde(default)]
    google_ai_usage: AiUsage,
    #[serde(default)]
    google_ai_requests: u64,
    #[serde(default)]
    google_ai_last_usage: Option<AiUsage>,
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
    session_queue: Vec<PathBuf>,
    #[serde(default)]
    session_current: Option<PathBuf>,
    #[serde(default)]
    session_position_seconds: f64,
    #[serde(default)]
    show_queue_on_start: bool,
    #[serde(default = "default_compact_mode")]
    compact_mode: bool,
    #[serde(default = "default_start_maximized")]
    start_maximized: bool,
    #[serde(default = "default_animations_enabled")]
    animations_enabled: bool,
    #[serde(default)]
    lyrics_enabled: bool,
    #[serde(default)]
    default_app_prompt_dismissed: bool,
    #[serde(default)]
    fetch_online_artwork: bool,
    #[serde(default)]
    online_artwork_prompt_seen: bool,
    #[serde(default)]
    developer_console_enabled: bool,
    #[serde(default = "default_true")]
    ai_audit_check_duplicates: bool,
    #[serde(default = "default_true")]
    ai_audit_check_artist_mismatches: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            welcome_completed: false,
            folders: Vec::new(),
            favorite_genres: HashSet::new(),
            liked_songs: HashSet::new(),
            disliked_songs: HashSet::new(),
            liked_artists: HashSet::new(),
            liked_albums: HashSet::new(),
            playlists: Vec::new(),
            history: Vec::new(),
            track_engagement: HashMap::new(),
            recommendation_feedback: HashMap::new(),
            search_history: Vec::new(),
            excluded_tracks: HashSet::new(),
            view_mode: ViewMode::default(),
            album_view_mode: ViewMode::default(),
            artist_view_mode: ViewMode::default(),
            accent: String::new(),
            shuffle: false,
            repeat: RepeatMode::default(),
            smart_queue_mode: SmartQueueMode::default(),
            use_listening_history: true,
            track_fade_seconds: 0.0,
            replay_gain_mode: ReplayGainMode::Off,
            discovery: default_discovery(),
            prefer_unplayed: default_prefer_unplayed(),
            recommendation_variety: default_recommendation_variety(),
            recommendation_size: default_recommendation_size(),
            ai_recommendations_enabled: false,
            google_ai_model: GeminiModel::default(),
            google_ai_thinking: GeminiThinking::default(),
            ai_mix_size: default_ai_mix_size(),
            ai_mix_include_liked: true,
            ai_mix_include_played: true,
            ai_mix_include_unplayed: true,
            google_ai_usage: AiUsage::default(),
            google_ai_requests: 0,
            google_ai_last_usage: None,
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
            session_queue: Vec::new(),
            session_current: None,
            session_position_seconds: 0.0,
            show_queue_on_start: false,
            compact_mode: default_compact_mode(),
            start_maximized: default_start_maximized(),
            animations_enabled: default_animations_enabled(),
            lyrics_enabled: false,
            default_app_prompt_dismissed: false,
            fetch_online_artwork: false,
            online_artwork_prompt_seen: false,
            developer_console_enabled: false,
            ai_audit_check_duplicates: true,
            ai_audit_check_artist_mismatches: true,
        }
    }
}

fn default_discovery() -> f32 {
    0.5
}

// Existing settings files predate onboarding and should be treated as completed.
fn legacy_welcome_completed() -> bool {
    true
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

fn default_ai_mix_size() -> usize {
    50
}

fn playback_fade_gain(position_seconds: f64, duration_seconds: Option<f64>, fade_seconds: f32) -> f32 {
    let fade_seconds = fade_seconds.clamp(0.0, 8.0) as f64;
    if fade_seconds <= f64::EPSILON {
        return 1.0;
    }
    let position = if position_seconds.is_finite() { position_seconds.max(0.0) } else { 0.0 };
    let duration = duration_seconds.filter(|duration| duration.is_finite() && *duration > 0.0);
    let edge = duration.map_or(fade_seconds, |duration| fade_seconds.min(duration * 0.5));
    let fade_in = (position / edge).clamp(0.0, 1.0);
    let fade_out = duration.map_or(1.0, |duration| ((duration - position) / edge).clamp(0.0, 1.0));
    fade_in.min(fade_out) as f32
}

fn crossfade_gains(progress: f32) -> (f32, f32) {
    let angle = progress.clamp(0.0, 1.0) * std::f32::consts::FRAC_PI_2;
    (angle.cos(), angle.sin())
}

fn replay_gain_multiplier_from_db(decibels: Option<f32>) -> f32 {
    decibels.map_or(1.0, |gain| {
        if gain.is_finite() {
            10.0_f32.powf(gain.clamp(-18.0, 6.0) / 20.0)
        } else {
            1.0
        }
    })
}

fn default_true() -> bool {
    true
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

fn default_start_maximized() -> bool {
    true
}

fn default_animations_enabled() -> bool {
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
    Mixes,
    Stats,
}

struct ScanResult {
    tracks: Vec<Track>,
    error: Option<String>,
    issues: Vec<String>,
    issue_count: usize,
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

struct OnlineArtworkResult {
    covers: Vec<(Vec<PathBuf>, Option<Arc<Vec<u8>>>)>,
    artist_covers: Vec<(String, Option<Arc<Vec<u8>>>)>,
    stage: &'static str,
    completed: usize,
    total: usize,
}

#[derive(Clone)]
enum SearchSuggestion {
    Song(usize),
    Artist(String),
    Album(String, usize),
}

#[derive(Clone)]
struct AiMixCandidate {
    index: usize,
    title: String,
    artist: String,
    genre: String,
    familiarity: &'static str,
    engagement: String,
}

type AiMixMessage = Result<(String, Vec<usize>, AiUsage, Option<String>), String>;

#[derive(Clone, Debug)]
struct AiAuditFinding {
    kind: String,
    track_ids: Vec<usize>,
    suggested_artist: String,
    explanation: String,
}

type AiAuditMessage = Result<(Vec<AiAuditFinding>, AiUsage), String>;

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

struct CrossfadeTransition {
    next_sink: Sink,
    next_stream: OutputStream,
    next_queue_position: usize,
    next_track_index: usize,
    duration: Duration,
}

// Owned snapshot of everything the recommendation / mix scoring needs, so the
// work can run on a background thread without touching the UI state.
struct MixContext {
    tracks: Vec<Track>,
    history: Vec<PathBuf>,
    track_engagement: HashMap<PathBuf, TrackEngagement>,
    recommendation_feedback: HashMap<PathBuf, i8>,
    use_listening_history: bool,
    liked_songs: HashSet<PathBuf>,
    disliked_songs: HashSet<PathBuf>,
    excluded_tracks: HashSet<PathBuf>,
    liked_artists: HashSet<String>,
    liked_albums: HashSet<String>,
    favorite_genres: HashSet<String>,
    discovery: f32,
    prefer_unplayed: bool,
    recommendation_variety: f32,
    recommendation_size: usize,
    home_genre: String,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
struct TrackEngagement {
    plays: u32,
    listened_seconds: f64,
    completions: u32,
    skips: u32,
    replays: u32,
    seek_backs: u32,
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

pub struct PlayerApp {
    welcome_open: bool,
    welcome_step: u8,
    welcome_completed: bool,
    tracks: Vec<Track>,
    folders: Vec<PathBuf>,
    liked_songs: HashSet<PathBuf>,
    disliked_songs: HashSet<PathBuf>,
    liked_artists: HashSet<String>,
    liked_albums: HashSet<String>,
    favorite_genres: HashSet<String>,
    playlists: Vec<Playlist>,
    history: Vec<PathBuf>,
    track_engagement: HashMap<PathBuf, TrackEngagement>,
    recommendation_feedback: HashMap<PathBuf, i8>,
    excluded_tracks: HashSet<PathBuf>,
    query: String,
    search_history: Vec<String>,
    quick_pick_page: usize,
    search_committed: bool,
    search_suggestions_open: bool,
    search_suggestion_selection: usize,
    search_ai_pending: bool,
    search_ai_results: Vec<usize>,
    search_ai_open: bool,
    search_ai_answer: String,
    search_suggestions_focused: bool,
    fetch_online_artwork: bool,
    online_artwork_prompt_seen: bool,
    online_artwork_prompt_open: bool,
    online_artwork_pending: bool,
    online_artwork_status: String,
    online_artwork_receiver: Receiver<OnlineArtworkResult>,
    online_artwork_sender: Sender<OnlineArtworkResult>,
    artist_artworks: HashMap<String, Arc<Vec<u8>>>,
    developer_console_enabled: bool,
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
    ai_recommendations_enabled: bool,
    google_ai_model: GeminiModel,
    google_ai_thinking: GeminiThinking,
    ai_mix_size: usize,
    ai_mix_include_liked: bool,
    ai_mix_include_played: bool,
    ai_mix_include_unplayed: bool,
    google_ai_usage: AiUsage,
    google_ai_requests: u64,
    google_ai_last_usage: Option<AiUsage>,
    ui_scale: f32,
    volume: f32,
    home_genre: String,
    discover_genre: String,
    discover_decade: Option<u32>,
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
    smart_queue_mode: SmartQueueMode,
    use_listening_history: bool,
    track_fade_seconds: f32,
    replay_gain_mode: ReplayGainMode,
    playlist_name: String,
    smart_playlist_rule: SmartPlaylistRule,
    renaming_playlist: Option<usize>,
    playlist_rename_draft: String,
    selected_playlist: Option<usize>,
    selected_artist: Option<String>,
    selected_album: Option<String>,
    settings_draft: Option<Preferences>,
    settings_advanced: bool,
    settings_category: String,
    pending_delete: Option<(PathBuf, String)>,
    pending_bulk_delete: Option<(Vec<PathBuf>, String)>,
    deleted_songs: Vec<DeletedSongRecord>,
    auto_purge_deleted: bool,
    clear_deleted_history_confirmation: bool,
    lyrics: Option<String>,
    lyrics_error: Option<String>,
    queue: Vec<usize>,
    resume_queue_paths: Vec<PathBuf>,
    resume_current_path: Option<PathBuf>,
    resume_position_seconds: f64,
    queue_collapsed: bool,
    now_playing_expanded: bool,
    now_playing_tab: NowPlayingTab,
    session_target_minutes: usize,
    compact_mode: bool,
    start_maximized: bool,
    animations_enabled: bool,
    lyrics_enabled: bool,
    current: Option<usize>,
    playback_offset: Duration,
    last_playback_sample: Option<(PathBuf, Duration)>,
    completed_session_path: Option<PathBuf>,
    last_engagement_save: Instant,
    sink: Option<Sink>,
    stream: Option<OutputStream>,
    crossfade: Option<CrossfadeTransition>,
    crossfade_unavailable_path: Option<PathBuf>,
    scan_receiver: Option<Receiver<ScanResult>>,
    scan_status: String,
    scan_issues: Vec<String>,
    scan_issue_count: usize,
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
    ai_mix_prompt: String,
    ai_mix_results: Vec<usize>,
    ai_recommendation_indices: Vec<usize>,
    ai_recommendation_mode: bool,
    ai_mix_pending: bool,
    ai_mix_status: String,
    ai_mix_receiver: Receiver<AiMixMessage>,
    ai_mix_sender: Sender<AiMixMessage>,
    ai_audit_receiver: Receiver<AiAuditMessage>,
    ai_audit_sender: Sender<AiAuditMessage>,
    ai_audit_check_duplicates: bool,
    ai_audit_check_artist_mismatches: bool,
    ai_audit_pending: bool,
    ai_audit_status: String,
    ai_audit_findings: Vec<AiAuditFinding>,
    google_ai_key_draft: String,
    google_ai_key_saved: bool,
    developer_output_open: bool,
    developer_output: VecDeque<String>,
    last_status_seen: String,
    splash_started: Instant,
    error_notification: Option<(String, Instant)>,
    update_receiver: Receiver<UpdateMessage>,
    update_sender: Sender<UpdateMessage>,
    update_available: Option<UpdateInfo>,
    update_downloading: bool,
    update_notice: Option<String>,
    release_notes_open: bool,
    last_animated_page: Page,
    page_animation_started: Instant,
    default_app_prompt_open: bool,
    pending_startup_file: Option<PathBuf>,
    #[cfg(target_os = "windows")]
    media_controls: Option<MediaControls>,
    #[cfg(target_os = "windows")]
    media_controls_initialized: bool,
    #[cfg(target_os = "windows")]
    published_media_state: Option<(Option<PathBuf>, bool)>,
}


impl PlayerApp {
    #[cfg(test)]
    fn new() -> Self {
        Self::new_with_folder(None, None)
    }

    pub fn new_with_folder(startup_folder: Option<PathBuf>, startup_file: Option<PathBuf>) -> Self {
        let (mut preferences, preference_error) = match load_preferences() {
            Ok(preferences) => (preferences, None),
            Err(error) => (Preferences::default(), Some(error)),
        };
        let startup_folder = startup_folder.or_else(|| {
            startup_file
                .as_ref()
                .filter(|file| file.is_file())
                .and_then(|file| file.parent().map(Path::to_path_buf))
        });
        let add_startup_folder = startup_folder
            .filter(|folder| folder.is_dir())
            .filter(|folder| !preferences.folders.contains(folder));
        if let Some(folder) = &add_startup_folder {
            preferences.folders.push(folder.clone());
        }
        let (artwork_sender, artwork_receiver) = mpsc::channel();
        let (media_key_sender, media_key_receiver) = mpsc::channel();
        let (ai_mix_sender, ai_mix_receiver) = mpsc::channel();
        let (ai_audit_sender, ai_audit_receiver) = mpsc::channel();
        let (online_artwork_sender, online_artwork_receiver) = mpsc::channel();
        let (update_sender, update_receiver) = mpsc::channel();
        let update_check_sender = update_sender.clone();
        thread::spawn(move || {
            let result = check_for_update();
            let _ = update_check_sender.send(UpdateMessage::Check(result));
        });
        let mut app = Self {
            tracks: Vec::new(),
            folders: preferences.folders,
            liked_songs: preferences.liked_songs,
            disliked_songs: preferences.disliked_songs,
            liked_artists: preferences.liked_artists,
            liked_albums: preferences.liked_albums,
            playlists: preferences.playlists,
            history: preferences.history,
            track_engagement: preferences.track_engagement,
            excluded_tracks: preferences.excluded_tracks,
            query: String::new(),
            search_history: preferences.search_history.into_iter().take(5).collect(),
            quick_pick_page: 0,
            search_committed: false,
            search_suggestions_open: false,
            search_suggestion_selection: 0,
            search_ai_pending: false,
            search_ai_results: Vec::new(),
            search_ai_open: false,
            search_ai_answer: String::new(),
            search_suggestions_focused: false,
            fetch_online_artwork: preferences.fetch_online_artwork,
            online_artwork_prompt_seen: preferences.online_artwork_prompt_seen,
            online_artwork_prompt_open: false,
            online_artwork_pending: false,
            online_artwork_status: String::new(),
            online_artwork_receiver,
            online_artwork_sender,
            artist_artworks: HashMap::new(),
            developer_console_enabled: preferences.developer_console_enabled,
            page: Page::Home,
            loaded_collection_page: None,
            loaded_collection_query: String::new(),
            loaded_collection_items: 50,
            view_mode: preferences.view_mode,
            album_view_mode: preferences.album_view_mode,
            artist_view_mode: preferences.artist_view_mode,
            accent: if preferences.accent.is_empty() {
                "Rose".into()
            } else {
                preferences.accent
            },
            discovery: preferences.discovery.clamp(0.0, 1.0),
            prefer_unplayed: preferences.prefer_unplayed,
            recommendation_variety: preferences.recommendation_variety.clamp(0.0, 1.0),
            recommendation_size: preferences.recommendation_size.clamp(20, 100),
            ai_recommendations_enabled: preferences.ai_recommendations_enabled,
            google_ai_model: preferences.google_ai_model,
            google_ai_thinking: preferences.google_ai_thinking,
            ai_mix_size: preferences.ai_mix_size.clamp(50, 200),
            ai_mix_include_liked: preferences.ai_mix_include_liked,
            ai_mix_include_played: preferences.ai_mix_include_played,
            ai_mix_include_unplayed: preferences.ai_mix_include_unplayed,
            google_ai_usage: preferences.google_ai_usage,
            google_ai_requests: preferences.google_ai_requests,
            google_ai_last_usage: preferences.google_ai_last_usage,
            ui_scale: preferences.ui_scale.clamp(75.0, 150.0),
            volume: preferences.volume.clamp(0.0, 1.0),
            home_genre: String::new(),
            discover_genre: String::new(),
            discover_decade: None,
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
            settings_advanced: false,
            settings_category: "Appearance".into(),
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
            start_maximized: preferences.start_maximized,
            animations_enabled: preferences.animations_enabled,
            lyrics_enabled: preferences.lyrics_enabled,
            current: None,
            playback_offset: Duration::ZERO,
            last_playback_sample: None,
            completed_session_path: None,
            last_engagement_save: Instant::now(),
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
            ai_mix_prompt: String::new(),
            ai_mix_results: Vec::new(),
            ai_recommendation_indices: Vec::new(),
            ai_recommendation_mode: false,
            ai_mix_pending: false,
            ai_mix_status: String::new(),
            ai_mix_receiver,
            ai_mix_sender,
            ai_audit_receiver,
            ai_audit_sender,
            ai_audit_check_duplicates: preferences.ai_audit_check_duplicates,
            ai_audit_check_artist_mismatches: preferences.ai_audit_check_artist_mismatches,
            ai_audit_pending: false,
            ai_audit_status: String::new(),
            ai_audit_findings: Vec::new(),
            google_ai_key_draft: String::new(),
            google_ai_key_saved: google_ai_key_exists(),
            developer_output_open: false,
            developer_output: VecDeque::new(),
            last_status_seen: String::new(),
            splash_started: Instant::now(),
            error_notification: None,
            update_receiver,
            update_sender,
            update_available: None,
            update_downloading: false,
            update_notice: None,
            release_notes_open: false,
            last_animated_page: Page::Home,
            page_animation_started: Instant::now(),
            default_app_prompt_open: !preferences.default_app_prompt_dismissed,
            pending_startup_file: startup_file.filter(|file| file.is_file()),
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
        if add_startup_folder.is_some() {
            app.status = "Added the File Explorer folder to your Lilt library.".into();
            app.save_state();
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

    fn poll_update_messages(&mut self) {
        while let Ok(message) = self.update_receiver.try_recv() {
            match message {
                UpdateMessage::Check(Ok(Some(update))) => self.update_available = Some(update),
                UpdateMessage::Check(Ok(None)) | UpdateMessage::Check(Err(_)) => {}
                UpdateMessage::Download(result) => {
                    self.update_downloading = false;
                    match result {
                        Ok(()) => {
                            self.update_available = None;
                            self.update_notice = Some(
                                "The installer was opened. Follow the Windows Installer prompts to update Lilt.".into(),
                            );
                        }
                        Err(error) => self.update_notice = Some(error),
                    }
                }
            }
        }
    }

    fn download_update(&mut self) {
        let Some(update) = self.update_available.clone() else {
            return;
        };
        self.update_downloading = true;
        let sender = self.update_sender.clone();
        thread::spawn(move || {
            let result = download_and_open_update(&update);
            let _ = sender.send(UpdateMessage::Download(result));
        });
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
                    // Media keys are system-wide. Wake egui even while another
                    // app has focus so the queued command is applied promptly.
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
            if icon_text_button(ui, self.ui_icons.get("next"), "Play next", false).clicked() {
                play_next = true;
                ui.close_menu();
            }
            if ui.button("♫  Add to queue").clicked() {
                enqueue = true;
                ui.close_menu();
            }
            if icon_text_button(
                ui,
                self.ui_icons.get("music_note"),
                "Start song radio",
                false,
            )
            .clicked()
            {
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
            if !is_unknown(&album)
                && icon_text_button(ui, self.ui_icons.get("album"), "Go to album", false).clicked()
            {
                open_album = true;
                ui.close_menu();
            }
            if artist.is_some()
                && icon_text_button(ui, self.ui_icons.get("person"), "Go to artist", false)
                    .clicked()
            {
                open_artist = true;
                ui.close_menu();
            }
            if !playlist_names.is_empty() {
                ui.menu_button("Add to playlist", |ui| {
                    for (playlist_index, name) in playlist_names.iter().enumerate() {
                        if ui.button(name).clicked() {
                            add_to_playlist = Some(playlist_index);
                            ui.close_menu();
                        }
                    }
                });
            }
            if icon_text_button(
                ui,
                self.ui_icons.get("add"),
                "Create playlist from song",
                false,
            )
            .clicked()
            {
                create_playlist = true;
                ui.close_menu();
            }
            ui.separator();
            if icon_text_button(ui, self.ui_icons.get("copy"), "Copy song details", false).clicked()
            {
                ui.ctx().copy_text(format!(
                    "{} — {}\nAlbum: {}\n{}",
                    title,
                    display_credit(&artist_credit),
                    display_metadata(&album),
                    path.display()
                ));
                ui.close_menu();
            }
            if icon_text_button(ui, self.ui_icons.get("copy"), "Copy file path", false).clicked() {
                ui.ctx().copy_text(path.to_string_lossy().into_owned());
                ui.close_menu();
            }
            if icon_text_button(
                ui,
                self.ui_icons.get("folder_open"),
                "Reveal file in Explorer",
                false,
            )
            .clicked()
            {
                #[cfg(target_os = "windows")]
                reveal_path_in_explorer(&path);
                ui.close_menu();
            }
            if icon_text_button(
                ui,
                self.ui_icons.get("history"),
                "Clear this song from history",
                false,
            )
            .clicked()
            {
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
            if icon_text_button(ui, self.ui_icons.get("shuffle"), "Shuffle album", false).clicked()
            {
                shuffle = true;
                ui.close_menu();
            }
            if icon_text_button(ui, self.ui_icons.get("next"), "Play album next", false).clicked() {
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
            if icon_text_button(ui, self.ui_icons.get("album"), "Open album page", false).clicked()
            {
                open = true;
                ui.close_menu();
            }
            if !playlist_names.is_empty() {
                ui.menu_button("Add album to playlist", |ui| {
                    for (playlist_index, name) in playlist_names.iter().enumerate() {
                        if ui.button(name).clicked() {
                            add_to_playlist = Some(playlist_index);
                            ui.close_menu();
                        }
                    }
                });
            }
            if icon_text_button(
                ui,
                self.ui_icons.get("add"),
                "Create playlist from album",
                false,
            )
            .clicked()
            {
                create_playlist = true;
                ui.close_menu();
            }
            ui.separator();
            if icon_text_button(
                ui,
                self.ui_icons.get("copy"),
                "Copy album and artist",
                false,
            )
            .clicked()
            {
                ui.ctx()
                    .copy_text(format!("{}\n{}", album, credits.join(", ")));
                ui.close_menu();
            }
            if icon_text_button(
                ui,
                self.ui_icons.get("copy"),
                "Copy all album file paths",
                false,
            )
            .clicked()
            {
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
            if icon_text_button(
                ui,
                self.ui_icons.get("history"),
                "Clear album listening history",
                false,
            )
            .clicked()
            {
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
            if icon_text_button(ui, self.ui_icons.get("shuffle"), "Shuffle artist", false).clicked()
            {
                shuffle = true;
                ui.close_menu();
            }
            if icon_text_button(ui, self.ui_icons.get("next"), "Play artist next", false).clicked()
            {
                enqueue = true;
                ui.close_menu();
            }
            if icon_text_button(
                ui,
                self.ui_icons.get("music_note"),
                "Start artist radio",
                false,
            )
            .clicked()
            {
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
            if icon_text_button(ui, self.ui_icons.get("person"), "Open artist page", false)
                .clicked()
            {
                open = true;
                ui.close_menu();
            }
            if !playlist_names.is_empty() {
                ui.menu_button("Add artist to playlist", |ui| {
                    for (playlist_index, name) in playlist_names.iter().enumerate() {
                        if ui.button(name).clicked() {
                            add_to_playlist = Some(playlist_index);
                            ui.close_menu();
                        }
                    }
                });
            }
            if icon_text_button(
                ui,
                self.ui_icons.get("add"),
                "Create playlist from artist",
                false,
            )
            .clicked()
            {
                create_playlist = true;
                ui.close_menu();
            }
            ui.separator();
            if icon_text_button(ui, self.ui_icons.get("copy"), "Copy artist name", false).clicked()
            {
                ui.ctx().copy_text(artist.clone());
                ui.close_menu();
            }
            if icon_text_button(
                ui,
                self.ui_icons.get("copy"),
                "Copy all artist file paths",
                false,
            )
            .clicked()
            {
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
            if icon_text_button(
                ui,
                self.ui_icons.get("history"),
                "Clear artist listening history",
                false,
            )
            .clicked()
            {
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
            if icon_text_button(ui, self.ui_icons.get("shuffle"), "Shuffle and play", false)
                .clicked()
            {
                shuffle = true;
                ui.close_menu();
            }
            if ui.button("♫  Add mix to queue").clicked() {
                enqueue = true;
                ui.close_menu();
            }
            if icon_text_button(ui, self.ui_icons.get("add"), "Save mix as playlist", false)
                .clicked()
            {
                create_playlist = true;
                ui.close_menu();
            }
            ui.separator();
            if icon_text_button(ui, self.ui_icons.get("copy"), "Copy mix track list", false)
                .clicked()
            {
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
            if icon_text_button(
                ui,
                self.ui_icons.get("history"),
                "Clear these tracks from history",
                false,
            )
            .clicked()
            {
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
                if let Some(open_file) = self.pending_startup_file.take() {
                    let wanted = fs::canonicalize(&open_file).unwrap_or(open_file.clone());
                    if let Some(index) = self.tracks.iter().position(|track| {
                        fs::canonicalize(&track.path)
                            .map(|path| path == wanted)
                            .unwrap_or_else(|_| track.path == open_file)
                    }) {
                        self.queue = vec![index];
                        self.play_queue_at(0);
                    } else {
                        self.status = format!(
                            "Could not open {} because it is not a supported audio file.",
                            open_file.display()
                        );
                    }
                }
                if self.tracks.iter().any(|track| track.artwork.is_none()) {
                    if !self.online_artwork_prompt_seen {
                        self.online_artwork_prompt_open = true;
                    } else if self.fetch_online_artwork {
                        self.start_online_artwork_fetch();
                    }
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

    fn start_online_artwork_fetch(&mut self) {
        if self.online_artwork_pending {
            return;
        }
        let mut albums = HashMap::<(String, String), (Vec<PathBuf>, String, String)>::new();
        for track in &self.tracks {
            if track.artwork.is_some() || is_unknown(&track.album) || is_unknown(&track.artist) {
                continue;
            }
            let key = (track.artist.to_lowercase(), track.album.to_lowercase());
            let entry = albums
                .entry(key)
                .or_insert_with(|| (Vec::new(), track.artist.clone(), track.album.clone()));
            entry.0.push(track.path.clone());
        }
        let jobs = albums.into_values().collect::<Vec<_>>();
        let mut artist_set = BTreeMap::<String, String>::new();
        for track in &self.tracks {
            for artist in artist_names(&track.artist) {
                artist_set.entry(artist.to_lowercase()).or_insert(artist);
            }
        }
        let artist_jobs = artist_set.into_values().collect::<Vec<_>>();
        let mut known_album_art_hashes = HashSet::new();
        for artwork in self
            .tracks
            .iter()
            .filter_map(|track| track.artwork.as_deref())
        {
            let mut hasher = DefaultHasher::new();
            artwork.hash(&mut hasher);
            known_album_art_hashes.insert(hasher.finish());
        }
        if jobs.is_empty() && artist_jobs.is_empty() {
            self.online_artwork_status = "No tagged albums or artists need cover art.".into();
            return;
        }
        let total = jobs.len();
        let artist_total = artist_jobs.len();
        self.online_artwork_pending = true;
        self.online_artwork_status =
            format!("Looking up album and artist covers in the background…");
        let sender = self.online_artwork_sender.clone();
        #[cfg(windows)]
        thread::spawn(move || {
            let mut known_album_art_hashes = known_album_art_hashes;
            let mut batch = Vec::with_capacity(50);
            for (position, (paths, artist, album)) in jobs.into_iter().enumerate() {
                let cache_path = online_artwork_cache_path(&artist, &album);
                let cached = cache_path
                    .as_ref()
                    .ok()
                    .and_then(|path| fs::read(path).ok())
                    .filter(|bytes| !bytes.is_empty() && bytes.len() <= MAX_CACHED_ARTWORK_BYTES);
                let bytes = if let Some(cached) = cached {
                    Some(cached)
                } else {
                    let result = lookup_online_album_art(&artist, &album)
                        .ok()
                        .filter(|bytes| {
                            !bytes.is_empty() && bytes.len() <= MAX_CACHED_ARTWORK_BYTES
                        });
                    // MusicBrainz requests are globally rate limited per client IP.
                    thread::sleep(Duration::from_millis(1_050));
                    result
                };
                if let (Some(bytes), Ok(cache_path)) =
                    (&bytes, online_artwork_cache_path(&artist, &album))
                {
                    if let Some(parent) = cache_path.parent() {
                        let _ = fs::create_dir_all(parent);
                    }
                    let _ = fs::write(cache_path, bytes);
                }
                if let Some(bytes) = &bytes {
                    let mut hasher = DefaultHasher::new();
                    bytes.hash(&mut hasher);
                    known_album_art_hashes.insert(hasher.finish());
                }
                batch.push((paths, bytes.map(Arc::new)));
                if batch.len() >= 50 || position + 1 == total {
                    let _ = sender.send(OnlineArtworkResult {
                        covers: std::mem::take(&mut batch),
                        artist_covers: Vec::new(),
                        stage: "albums",
                        completed: position + 1,
                        total,
                    });
                    batch = Vec::with_capacity(50);
                }
            }
            if total == 0 {
                let _ = sender.send(OnlineArtworkResult {
                    covers: Vec::new(),
                    artist_covers: Vec::new(),
                    stage: "albums",
                    completed: 0,
                    total: 0,
                });
            }
            let mut artist_batch = Vec::with_capacity(50);
            for (position, artist) in artist_jobs.into_iter().enumerate() {
                let cache_path = online_artist_art_cache_path(&artist);
                let mut cached = cache_path
                    .as_ref()
                    .ok()
                    .and_then(|path| fs::read(path).ok())
                    .filter(|bytes| !bytes.is_empty() && bytes.len() <= MAX_CACHED_ARTWORK_BYTES);
                if cached.as_ref().is_some_and(|bytes| {
                    let mut hasher = DefaultHasher::new();
                    bytes.hash(&mut hasher);
                    known_album_art_hashes.contains(&hasher.finish())
                }) {
                    cached = None;
                    if let Ok(path) = &cache_path {
                        let _ = fs::remove_file(path);
                    }
                }
                let bytes = if let Some(cached) = cached {
                    Some(cached)
                } else {
                    let result = lookup_online_artist_art(&artist).ok().filter(|bytes| {
                        if bytes.is_empty() || bytes.len() > MAX_CACHED_ARTWORK_BYTES {
                            return false;
                        }
                        let mut hasher = DefaultHasher::new();
                        bytes.hash(&mut hasher);
                        !known_album_art_hashes.contains(&hasher.finish())
                    });
                    thread::sleep(Duration::from_millis(1_050));
                    result
                };
                if let (Some(bytes), Ok(cache_path)) =
                    (&bytes, online_artist_art_cache_path(&artist))
                {
                    if let Some(parent) = cache_path.parent() {
                        let _ = fs::create_dir_all(parent);
                    }
                    let _ = fs::write(cache_path, bytes);
                }
                artist_batch.push((artist, bytes.map(Arc::new)));
                if artist_batch.len() >= 50 || position + 1 == artist_total {
                    let _ = sender.send(OnlineArtworkResult {
                        covers: Vec::new(),
                        artist_covers: std::mem::take(&mut artist_batch),
                        stage: "artists",
                        completed: position + 1,
                        total: artist_total,
                    });
                    artist_batch = Vec::with_capacity(50);
                }
            }
            let _ = sender.send(OnlineArtworkResult {
                covers: Vec::new(),
                artist_covers: Vec::new(),
                stage: "done",
                completed: 0,
                total: 0,
            });
        });
        #[cfg(not(windows))]
        {
            let _ = (jobs, artist_jobs, sender);
            self.online_artwork_pending = false;
            self.online_artwork_status =
                "Online cover lookup is available in the Windows build.".into();
        }
    }

    fn poll_online_artwork(&mut self, context: &egui::Context) {
        loop {
            match self.online_artwork_receiver.try_recv() {
                Ok(result) => {
                    for (paths, bytes) in result.covers {
                        if let Some(bytes) = bytes {
                            for path in &paths {
                                if let Some(track) =
                                    self.tracks.iter_mut().find(|track| &track.path == path)
                                {
                                    if track.artwork.is_none() {
                                        track.artwork = Some(Arc::clone(&bytes));
                                    }
                                }
                            }
                        }
                    }
                    for (artist, bytes) in result.artist_covers {
                        if let Some(bytes) = bytes {
                            self.artist_artworks.insert(artist.to_lowercase(), bytes);
                        }
                    }
                    match result.stage {
                        "albums" => {
                            self.online_artwork_status = format!(
                                "Album art lookup · {} / {} albums",
                                result.completed, result.total
                            )
                        }
                        "artists" => {
                            self.online_artwork_status = format!(
                                "Artist cover lookup · {} / {} artists",
                                result.completed, result.total
                            )
                        }
                        "done" => {
                            self.online_artwork_pending = false;
                            self.online_artwork_status =
                                "Album and artist cover lookup finished.".into();
                        }
                        _ => {}
                    }
                    context.request_repaint();
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.online_artwork_pending = false;
                    break;
                }
            }
        }
        if self.online_artwork_pending && context.input(|input| input.focused) {
            context.request_repaint_after(Duration::from_millis(400));
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
            track_engagement: self.track_engagement.clone(),
            search_history: self.search_history.clone(),
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
            ai_recommendations_enabled: self.ai_recommendations_enabled,
            google_ai_model: self.google_ai_model,
            google_ai_thinking: self.google_ai_thinking,
            ai_mix_size: self.ai_mix_size,
            ai_mix_include_liked: self.ai_mix_include_liked,
            ai_mix_include_played: self.ai_mix_include_played,
            ai_mix_include_unplayed: self.ai_mix_include_unplayed,
            google_ai_usage: self.google_ai_usage,
            google_ai_requests: self.google_ai_requests,
            google_ai_last_usage: self.google_ai_last_usage,
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
            start_maximized: self.start_maximized,
            animations_enabled: self.animations_enabled,
            lyrics_enabled: self.lyrics_enabled,
            default_app_prompt_dismissed: !self.default_app_prompt_open,
            fetch_online_artwork: self.fetch_online_artwork,
            online_artwork_prompt_seen: self.online_artwork_prompt_seen,
            developer_console_enabled: self.developer_console_enabled,
            ai_audit_check_duplicates: self.ai_audit_check_duplicates,
            ai_audit_check_artist_mismatches: self.ai_audit_check_artist_mismatches,
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

    fn show_topbar_file_menu(&mut self, ui: &mut egui::Ui) {
        ui.set_min_width(260.0);
        ui.label(
            RichText::new("LIBRARY")
                .small()
                .strong()
                .color(Color32::from_rgb(157, 164, 181)),
        );
        if icon_text_button(ui, self.ui_icons.get("folder_open"), "Add music folder…", false)
            .clicked()
        {
            self.add_folder();
            ui.close_menu();
        }
        if icon_text_button(ui, self.ui_icons.get("refresh"), "Rescan library", false)
            .clicked()
        {
            self.start_scan();
            ui.close_menu();
        }

        let current_track = self
            .current
            .and_then(|position| self.queue.get(position))
            .and_then(|&index| self.tracks.get(index))
            .map(|track| {
                (
                    track.title.clone(),
                    track.artist.clone(),
                    track.album.clone(),
                    track.path.clone(),
                )
            });
        if let Some((title, artist, album, path)) = current_track {
            ui.separator();
            ui.label(
                RichText::new("CURRENT TRACK")
                    .small()
                    .strong()
                    .color(Color32::from_rgb(157, 164, 181)),
            );
            if icon_text_button(ui, self.ui_icons.get("copy"), "Copy song details", false)
                .clicked()
            {
                ui.ctx().copy_text(format!(
                    "{} — {}\nAlbum: {}\n{}",
                    title,
                    display_credit(&artist),
                    display_metadata(&album),
                    path.display()
                ));
                ui.close_menu();
            }
            if icon_text_button(ui, self.ui_icons.get("copy"), "Copy file path", false).clicked() {
                ui.ctx().copy_text(path.to_string_lossy().into_owned());
                ui.close_menu();
            }
            if icon_text_button(
                ui,
                self.ui_icons.get("folder_open"),
                "Reveal in File Explorer",
                false,
            )
            .clicked()
            {
                #[cfg(target_os = "windows")]
                reveal_path_in_explorer(&path);
                ui.close_menu();
            }
        }

        ui.separator();
        if ui
            .button(RichText::new("Quit Lilt").color(Color32::from_rgb(220, 150, 158)))
            .clicked()
        {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn show_topbar_playback_menu(&mut self, ui: &mut egui::Ui) {
        ui.set_min_width(240.0);
        let playing = self
            .sink
            .as_ref()
            .is_some_and(|sink| !sink.is_paused() && !sink.empty());
        if icon_text_button(
            ui,
            self.ui_icons
                .get(if playing { "pause" } else { "play" }),
            if playing {
                "Pause · Space"
            } else {
                "Play · Space"
            },
            false,
        )
        .clicked()
        {
            self.toggle_playback();
            ui.close_menu();
        }
        if icon_text_button(ui, self.ui_icons.get("previous"), "Previous track", false).clicked() {
            self.previous_track();
            ui.close_menu();
        }
        if icon_text_button(ui, self.ui_icons.get("next"), "Next track", false).clicked() {
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
    }

    fn show_topbar_view_menu(&mut self, ui: &mut egui::Ui) {
        ui.set_min_width(260.0);
        ui.label(
            RichText::new("ZOOM")
                .small()
                .strong()
                .color(Color32::from_rgb(157, 164, 181)),
        );
        let mut preferences_changed = false;
        ui.horizontal(|ui| {
            if ui.small_button("−").on_hover_text("Zoom out").clicked() {
                self.ui_scale = (self.ui_scale - 5.0).max(75.0);
                preferences_changed = true;
            }
            ui.label(format!("{}%", self.ui_scale.round() as u32));
            if ui.small_button("+").on_hover_text("Zoom in").clicked() {
                self.ui_scale = (self.ui_scale + 5.0).min(150.0);
                preferences_changed = true;
            }
            if ui.small_button("Reset").clicked() {
                self.ui_scale = default_ui_scale();
                preferences_changed = true;
            }
        });
        ui.separator();
        preferences_changed |= ui
            .checkbox(&mut self.compact_mode, "Compact spacing")
            .changed();
        preferences_changed |= ui
            .checkbox(&mut self.animations_enabled, "Smooth animations")
            .changed();
        if preferences_changed {
            self.save_state();
        }
        ui.separator();
        if ui
            .button(if self.now_playing_expanded {
                "Close Now Playing"
            } else {
                "Open Now Playing"
            })
            .clicked()
        {
            self.now_playing_expanded = !self.now_playing_expanded;
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
        if self.developer_console_enabled
            && ui
                .selectable_label(self.developer_output_open, "Developer Output")
                .clicked()
        {
            self.developer_output_open = !self.developer_output_open;
            ui.close_menu();
        }
    }

    fn show_topbar_help_menu(&mut self, ui: &mut egui::Ui) {
        if ui.button("What’s new").clicked() {
            self.release_notes_open = true;
            ui.close_menu();
        }
        ui.label(RichText::new("Keyboard shortcuts").strong());
        ui.label("Space or Ctrl+P · Play or pause");
        ui.label("Left or Right arrow · Previous or next track");
        ui.label("Ctrl+F or / · Search");
        ui.label("Tab navigation · Up or Down moves through live search matches");
        ui.label("Ctrl+1…9 · Home, Songs, Albums, Artists, Liked, Playlists, History, Mixes, Settings");
        ui.label("Ctrl+R · Rescan   Ctrl+O · Add folder   Ctrl+L · Like   Ctrl+Q · Queue");
        ui.label("Alt+Up or Down · Volume");
        ui.separator();
        ui.label(format!("Lilt Player {}", env!("CARGO_PKG_VERSION")));
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
            Page::Discover | Page::Mixes | Page::Stats => Vec::new(),
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

    fn load_lyrics_for(&mut self, track_path: &Path) {
        match read_lyrics(track_path) {
            Ok(lyrics) => {
                self.lyrics = lyrics;
                self.lyrics_error = None;
            }
            Err(error) => {
                self.lyrics = None;
                self.lyrics_error = Some(error);
            }
        }
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
        let replay = self
            .current
            .and_then(|position| self.queue.get(position))
            .and_then(|&index| self.tracks.get(index))
            .is_some_and(|previous| previous.path == track_path);
        if self.lyrics_enabled {
            self.load_lyrics_for(&track_path);
        } else {
            self.lyrics = None;
            self.lyrics_error = None;
        }
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
        let source = match open_audio_source(&track_path) {
            Ok(source) => source,
            Err(error) => {
                self.status = format!("Could not decode {}: {error}", track_path.display());
                return;
            }
        };
        sink.set_volume(self.volume);
        sink.append(source);
        self.sink = Some(sink);
        self.stream = Some(stream);
        self.current = Some(queue_position);
        self.playback_offset = Duration::ZERO;
        self.last_playback_sample = Some((track_path.clone(), Duration::ZERO));
        self.completed_session_path = None;
        let engagement = self.track_engagement.entry(track_path.clone()).or_default();
        engagement.plays = engagement.plays.saturating_add(1);
        if replay {
            engagement.replays = engagement.replays.saturating_add(1);
        }
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
        let previous_position = self.playback_offset
            + self.sink.as_ref().map_or(Duration::ZERO, Sink::get_pos);
        if position + Duration::from_secs(3) < previous_position {
            let engagement = self.track_engagement.entry(path.clone()).or_default();
            engagement.seek_backs = engagement.seek_backs.saturating_add(1);
            self.recommendations_dirty = true;
            self.save_state();
        }
        let was_paused = self.sink.as_ref().is_some_and(Sink::is_paused);

        let is_opus = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("opus"));
        let mut decoder = if is_opus {
            None
        } else {
            let file = match File::open(&path) {
                Ok(file) => file,
                Err(error) => {
                    self.status = format!("Could not reopen {} to seek: {error}", path.display());
                    return;
                }
            };
            match Decoder::new(BufReader::new(file)) {
                Ok(decoder) => Some(decoder),
                Err(error) => {
                    self.status = format!("Could not decode {} to seek: {error}", path.display());
                    return;
                }
            }
        };
        let decoder_seeks = decoder
            .as_mut()
            .is_some_and(|decoder| decoder.try_seek(position).is_ok());

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
            sink.append(decoder.expect("decoder exists when native seek succeeded"));
        } else if is_opus {
            match open_audio_source(&path) {
                Ok(source) => sink.append(source.skip_duration(position)),
                Err(error) => {
                    self.status = format!("Could not decode {} to seek: {error}", path.display());
                    return;
                }
            }
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
        self.advance_track(true);
    }

    fn next_track_automatically(&mut self) {
        self.advance_track(false);
    }

    fn advance_track(&mut self, user_skipped: bool) {
        if self.repeat == RepeatMode::One {
            if let Some(current) = self.current {
                self.play_queue_at(current);
                return;
            }
        }
        if user_skipped {
            if let Some((path, progress)) = self.current_playback_path_and_position() {
                let duration = self
                    .tracks
                    .iter()
                    .find(|track| track.path == path)
                    .map(|track| track.duration)
                    .unwrap_or_default();
                if duration > 15.0 && progress.as_secs_f64() / duration < 0.58 {
                    let engagement = self.track_engagement.entry(path).or_default();
                    engagement.skips = engagement.skips.saturating_add(1);
                    self.recommendations_dirty = true;
                    self.save_state();
                }
            }
        }
        let next = self.current.map_or(0, |current| current + 1);
        if next < self.queue.len() {
            self.play_queue_at(next);
        } else if self.repeat == RepeatMode::All && !self.queue.is_empty() {
            self.play_queue_at(0);
        }
    }

    fn current_playback_path_and_position(&self) -> Option<(PathBuf, Duration)> {
        let position = self.current?;
        let index = *self.queue.get(position)?;
        let track = self.tracks.get(index)?;
        let path = track.path.clone();
        let elapsed = if self.sink.as_ref().is_some_and(Sink::empty) && track.duration > 0.0 {
            Duration::from_secs_f64(track.duration)
        } else {
            self.playback_offset + self.sink.as_ref().map_or(Duration::ZERO, Sink::get_pos)
        };
        Some((path, elapsed))
    }

    fn record_playback_sample(&mut self) {
        let Some((path, position)) = self.current_playback_path_and_position() else {
            self.last_playback_sample = None;
            return;
        };
        let playing = self.sink.as_ref().is_some_and(|sink| !sink.is_paused());
        if playing {
            if let Some((last_path, last_position)) = &self.last_playback_sample {
                if last_path == &path {
                    let delta = position.saturating_sub(*last_position);
                    let max_gap = if self.sink.as_ref().is_some_and(Sink::empty) {
                        Duration::from_secs(3_600)
                    } else {
                        Duration::from_secs(2)
                    };
                    if delta <= max_gap {
                        let engagement = self.track_engagement.entry(path.clone()).or_default();
                        engagement.listened_seconds += delta.as_secs_f64();
                    }
                }
            }
            let duration = self
                .tracks
                .iter()
                .find(|track| track.path == path)
                .map(|track| track.duration)
                .unwrap_or_default();
            if duration > 0.0
                && position.as_secs_f64() / duration >= 0.95
                && self.completed_session_path.as_ref() != Some(&path)
            {
                    let engagement = self.track_engagement.entry(path.clone()).or_default();
                    engagement.completions = engagement.completions.saturating_add(1);
                    self.completed_session_path = Some(path.clone());
                    self.recommendations_dirty = true;
            }
        }
        self.last_playback_sample = Some((path, position));
        if playing && self.last_engagement_save.elapsed() >= Duration::from_secs(12) {
            self.last_engagement_save = Instant::now();
            self.save_state();
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
                        ui.add(
                            egui::Image::new((cover.id(), Vec2::splat(art_size)))
                                .corner_radius(egui::CornerRadius::same(14)),
                        );
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
                        let icon_button = |icon: &str, fallback: &str| {
                            let button = if let Some(texture) = self.ui_icons.get(icon) {
                                egui::Button::image(
                                    egui::Image::new(texture).fit_to_exact_size(Vec2::splat(18.0)),
                                )
                            } else {
                                egui::Button::new(fallback)
                            };
                            button.min_size(Vec2::new(40.0, 40.0)).frame(false)
                        };
                        if ui
                            .add(icon_button("more", "⋮"))
                            .on_hover_text("Add to queue")
                            .clicked()
                        {
                            queue = true;
                        }
                        if self.page == Page::Playlists
                            && self.selected_playlist.is_some()
                            && ui
                                .add(icon_button("add", "+"))
                                .on_hover_text("Add to playlist")
                                .clicked()
                        {
                            add_to_playlist = true;
                        }
                        if ui
                            .add(icon_button("library_books", "Library"))
                            .on_hover_text("Remove from library; keeps the file")
                            .clicked()
                        {
                            remove = true;
                        }
                        if ui
                            .add(icon_button("delete", "Delete"))
                            .on_hover_text("Delete audio file permanently")
                            .clicked()
                        {
                            delete_file = true;
                        }
                        let dislike_button = if let Some(texture) = self.ui_icons.get("dislike") {
                            egui::Button::image(
                                egui::Image::new(texture)
                                    .tint(if disliked {
                                        Color32::from_rgb(255, 89, 100)
                                    } else {
                                        Color32::from_rgb(166, 171, 184)
                                    })
                                    .fit_to_exact_size(Vec2::splat(18.0)),
                            )
                            .min_size(Vec2::new(40.0, 40.0))
                            .frame(false)
                        } else {
                            egui::Button::new("👎")
                                .min_size(Vec2::new(40.0, 40.0))
                                .frame(false)
                        };
                        if ui.add(dislike_button).on_hover_text("Dislike").clicked() {
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
                        if ui
                            .add(icon_button("play", "▶"))
                            .on_hover_text("Play")
                            .clicked()
                        {
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
        let mut queue_again_track = None;
        let mut queue_again_all = Vec::new();
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
                    .selectable_label(
                        self.now_playing_tab == NowPlayingTab::Visualizer,
                        "Visualizer",
                    )
                    .clicked()
                {
                    self.now_playing_tab = NowPlayingTab::Visualizer;
                }
                if self.lyrics_enabled
                    && ui
                        .selectable_label(self.now_playing_tab == NowPlayingTab::Lyrics, "Lyrics")
                        .clicked()
                {
                    self.now_playing_tab = NowPlayingTab::Lyrics;
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
                                    egui::Image::new(&self.ui_icons["like"])
                                        .tint(if self.liked_songs.contains(&track.path) {
                                            accent
                                        } else {
                                            Color32::from_rgb(166, 171, 184)
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
                                        .tint(if self.disliked_songs.contains(&track.path) {
                                            Color32::from_rgb(255, 89, 100)
                                        } else {
                                            Color32::from_rgb(166, 171, 184)
                                        })
                                        .fit_to_exact_size(Vec2::splat(22.0)),
                                )
                                .frame(false),
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
                        let played_items = self
                            .queue
                            .iter()
                            .enumerate()
                            .take(start)
                            .filter_map(|(position, &index)| {
                                self.tracks
                                    .get(index)
                                    .cloned()
                                    .map(|track| (position, index, track))
                            })
                            .collect::<Vec<_>>();
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
                            if !played_items.is_empty() && ui.small_button("Queue again").clicked()
                            {
                                queue_again_all =
                                    played_items.iter().map(|(_, index, _)| *index).collect();
                            }
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
                                                    let title_response = ui.selectable_label(
                                                        selected,
                                                        RichText::new(&track.title).strong(),
                                                    );
                                                    if title_response.clicked() {
                                                        play_queue_position = Some(position);
                                                    }
                                                    self.show_track_context_menu(
                                                        &title_response,
                                                        index,
                                                    );
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
                                if !played_items.is_empty() {
                                    ui.add_space(14.0);
                                    ui.separator();
                                    ui.horizontal(|ui| {
                                        ui.heading("Queue again");
                                        ui.label(
                                            RichText::new("Previously played")
                                                .small()
                                                .color(Color32::GRAY),
                                        );
                                    });
                                    for (position, index, track) in played_items {
                                        ui.horizontal(|ui| {
                                            if let Some(texture) =
                                                self.artwork_texture(ui.ctx(), index)
                                            {
                                                ui.add(
                                                    egui::Image::new(&texture)
                                                        .fit_to_exact_size(Vec2::splat(38.0))
                                                        .corner_radius(egui::CornerRadius::same(5)),
                                                );
                                            } else {
                                                let (rect, _) = ui.allocate_exact_size(
                                                    Vec2::splat(38.0),
                                                    egui::Sense::hover(),
                                                );
                                                paint_cover_or_placeholder(ui, None, rect, index);
                                            }
                                            let response = ui.selectable_label(
                                                false,
                                                RichText::new(format!(
                                                    "{} · {}",
                                                    track.title,
                                                    display_credit(&track.artist)
                                                )),
                                            );
                                            if response.clicked() {
                                                play_queue_position = Some(position);
                                            }
                                            self.show_track_context_menu(&response, index);
                                            if ui.small_button("Queue again").clicked() {
                                                queue_again_track = Some(index);
                                            }
                                        });
                                    }
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
                    NowPlayingTab::Visualizer => self.show_visualizer(ui, current_index),
                    NowPlayingTab::Lyrics => self.show_expanded_lyrics(ui),
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
        if let Some(index) = queue_again_track {
            self.queue_track_next(index);
        }
        for index in queue_again_all.into_iter().rev() {
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

    fn show_visualizer(&self, ui: &mut egui::Ui, current_index: Option<usize>) {
        ui.heading("Visualizer");
        ui.label(
            RichText::new("A gentle animated pulse that follows playback.")
                .small()
                .color(Color32::GRAY),
        );
        ui.add_space(12.0);
        let (rect, _) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), ui.available_height().max(220.0)),
            egui::Sense::hover(),
        );
        ui.painter().rect_filled(
            rect,
            egui::CornerRadius::same(12),
            Color32::from_rgb(11, 13, 20),
        );
        let playing = self
            .sink
            .as_ref()
            .is_some_and(|sink| !sink.is_paused() && !sink.empty());
        let focused = ui.ctx().input(|input| input.focused);
        let phase = if self.animations_enabled && playing && focused {
            ui.ctx().input(|input| input.time) as f32
        } else {
            0.0
        };
        let bars = 48;
        let gap = 3.0;
        let bar_width = (rect.width() - gap * (bars as f32 - 1.0)) / bars as f32;
        let seed = current_index.unwrap_or(0) as f32 * 0.17;
        for bar in 0..bars {
            let center_weight = 1.0 - ((bar as f32 / (bars - 1) as f32) - 0.5).abs() * 1.6;
            let wave = ((phase * 2.6 + bar as f32 * 0.42 + seed).sin() * 0.5 + 0.5) * 0.65 + 0.25;
            let height =
                (rect.height() * center_weight.max(0.18) * wave).clamp(10.0, rect.height() - 24.0);
            let x = rect.left() + bar as f32 * (bar_width + gap);
            let bar_rect = egui::Rect::from_min_size(
                egui::pos2(x, rect.center().y - height * 0.5),
                Vec2::new(bar_width.max(2.0), height),
            );
            ui.painter().rect_filled(
                bar_rect,
                egui::CornerRadius::same(3),
                accent_color(&self.accent).linear_multiply(0.45 + wave * 0.55),
            );
        }
        if playing && self.animations_enabled && focused {
            ui.ctx().request_repaint_after(Duration::from_millis(40));
        }
    }

    fn show_expanded_lyrics(&self, ui: &mut egui::Ui) {
        ui.heading("Lyrics");
        if let Some(error) = &self.lyrics_error {
            ui.colored_label(Color32::LIGHT_RED, error);
        } else if let Some(lyrics) = &self.lyrics {
            egui::ScrollArea::vertical()
                .id_salt("expanded-lyrics")
                .show(ui, |ui| {
                    ui.label(RichText::new(lyrics).size(18.0).line_height(Some(28.0)));
                });
        } else {
            ui.label("No .lrc sidecar found for this song.");
            ui.label(
                RichText::new("Enable .lrc lyrics in Advanced Settings to use local lyric files.")
                    .small()
                    .color(Color32::GRAY),
            );
        }
    }

    fn show_smart_mixes(&mut self, ui: &mut egui::Ui) {
        let recommendations = self.recommendation_indices();
        let history_paths = self.history.iter().cloned().collect::<HashSet<_>>();
        let valid = |index: &usize| {
            self.tracks.get(*index).is_some_and(|track| {
                !self.excluded_tracks.contains(&track.path)
                    && !self.disliked_songs.contains(&track.path)
            })
        };
        let for_you = recommendations
            .iter()
            .copied()
            .filter(valid)
            .collect::<Vec<_>>();
        let familiar = self
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, track)| {
                !self.excluded_tracks.contains(&track.path)
                    && !self.disliked_songs.contains(&track.path)
                    && (self.liked_songs.contains(&track.path)
                        || history_paths.contains(&track.path))
            })
            .map(|(index, _)| index)
            .take(80)
            .collect::<Vec<_>>();
        let discovered = recommendations
            .iter()
            .copied()
            .filter(valid)
            .filter(|&index| {
                self.tracks.get(index).is_some_and(|track| {
                    !history_paths.contains(&track.path) && !self.liked_songs.contains(&track.path)
                })
            })
            .collect::<Vec<_>>();

        let mixes = vec![
            (
                "For you".to_string(),
                "A balanced mix from your likes and listening history.".to_string(),
                for_you,
            ),
            (
                "Familiar favorites".to_string(),
                "Songs and artists you return to most.".to_string(),
                familiar,
            ),
            (
                "Fresh discoveries".to_string(),
                "Personalized picks you haven't played or liked yet.".to_string(),
                discovered,
            ),
        ];
        let mut genre_tracks = BTreeMap::<String, Vec<usize>>::new();
        for &index in &recommendations {
            if let Some(track) = self.tracks.get(index) {
                let genre = track.genre.trim();
                if !genre.is_empty() && valid(&index) {
                    genre_tracks
                        .entry(genre.to_string())
                        .or_default()
                        .push(index);
                }
            }
        }

        ui.label(
            RichText::new(
                "Local recommendations that adapt to your likes, plays, and music metadata.",
            )
            .color(Color32::GRAY),
        );
        if self.ai_recommendations_enabled {
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        !self.ai_mix_pending,
                        egui::Button::new(if self.ai_mix_pending {
                            "Updating…"
                        } else {
                            "✦ Improve For you with AI"
                        }),
                    )
                    .clicked()
                {
                    self.request_ai_recommendations(ui.ctx());
                }
                ui.label(RichText::new("Runs only when clicked; sends up to 48 short song labels with liked/played/new tags.").small().color(Color32::GRAY));
            });
            if !self.ai_mix_status.is_empty() {
                ui.label(
                    RichText::new(&self.ai_mix_status)
                        .small()
                        .color(accent_color(&self.accent)),
                );
            }
        }
        ui.add_space(12.0);
        self.show_mix_studio(ui);
        ui.add_space(18.0);
        let mut play_mix = None;
        let mut queue_mix = None;
        ui.columns(3, |columns| {
            for (column, (title, description, tracks)) in columns.iter_mut().zip(mixes.iter()) {
                egui::Frame::new()
                    .fill(Color32::from_rgb(18, 20, 28))
                    .stroke(egui::Stroke::new(1.0_f32, Color32::from_rgb(40, 44, 56)))
                    .corner_radius(egui::CornerRadius::same(12))
                    .inner_margin(egui::Margin::same(12))
                    .show(column, |ui| {
                        let cover = tracks
                            .first()
                            .and_then(|&index| self.artwork_texture(ui.ctx(), index));
                        let (rect, _) = ui.allocate_exact_size(
                            Vec2::splat(ui.available_width().min(150.0)),
                            egui::Sense::hover(),
                        );
                        paint_cover_or_placeholder(
                            ui,
                            cover.as_ref(),
                            rect,
                            tracks.first().copied().unwrap_or(0),
                        );
                        ui.add_space(8.0);
                        ui.heading(title);
                        ui.label(
                            RichText::new(format!("{} songs", tracks.len()))
                                .small()
                                .color(accent_color(&self.accent)),
                        );
                        ui.label(RichText::new(description).small().color(Color32::GRAY));
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            if ui
                                .add_enabled(!tracks.is_empty(), egui::Button::new("▶  Play mix"))
                                .clicked()
                            {
                                play_mix = Some(tracks.clone());
                            }
                            if ui
                                .add_enabled(!tracks.is_empty(), egui::Button::new("Add to queue"))
                                .clicked()
                            {
                                queue_mix = Some(tracks.clone());
                            }
                        });
                    });
            }
        });
        if !genre_tracks.is_empty() {
            ui.add_space(18.0);
            ui.heading("Genre radios");
            ui.label(
                RichText::new("Each radio stays inside the genres in your library.")
                    .small()
                    .color(Color32::GRAY),
            );
            ui.horizontal_wrapped(|ui| {
                for (genre, tracks) in genre_tracks.iter().take(12) {
                    if ui
                        .add_enabled(
                            !tracks.is_empty(),
                            egui::Button::new(format!("♫  {genre} · {} songs", tracks.len())),
                        )
                        .clicked()
                    {
                        play_mix = Some(tracks.clone());
                    }
                }
            });
        }
        if let Some(tracks) = play_mix {
            self.play_indices(tracks, 0);
        }
        if let Some(tracks) = queue_mix {
            self.add_tracks_to_queue(&tracks);
        }
    }

    fn show_mix_studio(&mut self, ui: &mut egui::Ui) {
        egui::Frame::new()
            .fill(Color32::from_rgb(21, 23, 34))
            .stroke(egui::Stroke::new(1.0_f32, Color32::from_rgb(53, 66, 112)))
            .corner_radius(egui::CornerRadius::same(12))
            .inner_margin(egui::Margin::same(14))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("Mix Studio");
                    ui.label(RichText::new("Optional · Google AI").small().color(Color32::GRAY));
                });
                ui.label(RichText::new("Describe a mood, artist, or genre. Lilt sends compact title, artist, genre, and familiarity labels for available tracks so the mix can draw from your full library.").small().color(Color32::GRAY));
                egui::CollapsingHeader::new("Mix options")
                    .default_open(false)
                    .show(ui, |ui| {
                        ui.label(RichText::new("Choose what can appear in an AI mix").strong());
                        ui.horizontal_wrapped(|ui| {
                            let all_music = self.ai_mix_include_liked
                                && self.ai_mix_include_played
                                && self.ai_mix_include_unplayed;
                            if ui.selectable_label(all_music, "All music").clicked() && !all_music {
                                self.ai_mix_include_liked = true;
                                self.ai_mix_include_played = true;
                                self.ai_mix_include_unplayed = true;
                                self.save_state();
                            }
                            let mut changed = false;
                            changed |= ui.checkbox(&mut self.ai_mix_include_liked, "Liked music").changed();
                            changed |= ui.checkbox(&mut self.ai_mix_include_played, "Played music").changed();
                            changed |= ui.checkbox(&mut self.ai_mix_include_unplayed, "Unplayed music").changed();
                            if changed { self.save_state(); }
                        });
                        ui.label(RichText::new("Disliked and excluded tracks are always left out.").small().color(Color32::GRAY));
                        ui.separator();
                        ui.horizontal_wrapped(|ui| {
                            ui.label("Model");
                            let model_changed = egui::ComboBox::from_id_salt("mix-studio-model")
                                .selected_text(self.google_ai_model.label())
                                .show_ui(ui, |ui| {
                                    let mut changed = false;
                                    changed |= ui.selectable_value(&mut self.google_ai_model, GeminiModel::FlashLite, GeminiModel::FlashLite.label()).changed();
                                    changed |= ui.selectable_value(&mut self.google_ai_model, GeminiModel::Flash, GeminiModel::Flash.label()).changed();
                                    changed
                                }).inner.unwrap_or(false);
                            if model_changed { self.save_state(); }
                            ui.label("Thinking");
                            let thinking_changed = egui::ComboBox::from_id_salt("mix-studio-thinking")
                                .selected_text(match self.google_ai_thinking { GeminiThinking::Minimal => "Minimal · lowest usage", GeminiThinking::Low => "Low · more planning" })
                                .show_ui(ui, |ui| {
                                    let mut changed = false;
                                    changed |= ui.selectable_value(&mut self.google_ai_thinking, GeminiThinking::Minimal, "Minimal · lowest usage").changed();
                                    changed |= ui.selectable_value(&mut self.google_ai_thinking, GeminiThinking::Low, "Low · more planning").changed();
                                    changed
                                }).inner.unwrap_or(false);
                            if thinking_changed { self.save_state(); }
                            ui.label("Tracks");
                            let size_changed = egui::ComboBox::from_id_salt("mix-studio-size")
                                .selected_text(self.ai_mix_size.to_string())
                                .show_ui(ui, |ui| {
                                    let mut changed = false;
                                    for size in [50, 80, 100, 200] {
                                        changed |= ui.selectable_value(&mut self.ai_mix_size, size, size.to_string()).changed();
                                    }
                                    changed
                                }).inner.unwrap_or(false);
                            if size_changed { self.save_state(); }
                        });
                    });
                if ui.checkbox(&mut self.ai_recommendations_enabled, "Use AI when I refresh For you recommendations (off by default)").changed() {
                    if !self.ai_recommendations_enabled { self.ai_recommendation_indices.clear(); }
                    self.save_state();
                }
                ui.label(RichText::new("AI runs only when clicked. Google receives compact title, artist, genre, and liked/played/new labels for available tracks; no paths or audio. Full-library requests use more tokens.").small().color(Color32::GRAY));
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.ai_mix_prompt).desired_width(ui.available_width() - 130.0).hint_text("e.g. energetic electronic for a night drive"));
                    if ui.add_enabled(!self.ai_mix_pending, egui::Button::new(if self.ai_mix_pending { "Making…" } else { "Create mix" })).clicked() {
                        self.request_ai_mix(ui.ctx());
                    }
                });
                if !self.google_ai_key_saved {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Add your own Google AI key in Settings → AI.").small().color(Color32::GRAY));
                        if ui.small_button("Open settings").clicked() { self.page = Page::Settings; self.settings_category = "AI".into(); self.settings_advanced = true; }
                    });
                }
                if !self.ai_mix_status.is_empty() { ui.label(RichText::new(&self.ai_mix_status).small().color(accent_color(&self.accent))); }
                egui::CollapsingHeader::new("Google AI usage")
                    .default_open(false)
                    .show(ui, |ui| {
                ui.separator();
                ui.label(RichText::new("Usage reported to Lilt").strong());
                if let Some(usage) = self.google_ai_last_usage {
                    ui.label(format!("Last request · input {} · output {} · thinking {} · total {} tokens", usage.prompt_tokens, usage.output_tokens, usage.thinking_tokens, usage.total_tokens));
                } else {
                    ui.label("Last request · no usage data yet");
                }
                ui.label(format!("Lilt total · {} requests · {} input · {} output · {} thinking · {} total tokens", self.google_ai_requests, self.google_ai_usage.prompt_tokens, self.google_ai_usage.output_tokens, self.google_ai_usage.thinking_tokens, self.google_ai_usage.total_tokens));
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Google does not return remaining project quota to this app.").small().color(Color32::GRAY));
                    ui.hyperlink_to("View Google AI usage and limits", "https://aistudio.google.com/usage");
                });
                    });
                if !self.ai_mix_results.is_empty() {
                    ui.horizontal(|ui| {
                        ui.label(format!("{} picks", self.ai_mix_results.len()));
                        if ui.button("▶ Play").clicked() { self.play_indices(self.ai_mix_results.clone(), 0); }
                        if ui.button("Add to queue").clicked() { self.add_tracks_to_queue(&self.ai_mix_results.clone()); }
                        if ui.button("Save as playlist").clicked() { self.store_ai_mix_as_playlist(); }
                    });
                    egui::ScrollArea::vertical().id_salt("mix-studio-picks").max_height(180.0).show(ui, |ui| {
                        for &index in self.ai_mix_results.clone().iter().take(24) {
                            if let Some(track) = self.tracks.get(index) {
                                ui.label(RichText::new(format!("{} · {}", track.title, display_credit(&track.artist))).small());
                            }
                        }
                    });
                }
            });
    }

    fn show_listening_stats(&mut self, ui: &mut egui::Ui) {
        let mut play_counts = HashMap::<PathBuf, usize>::new();
        for path in &self.history {
            *play_counts.entry(path.clone()).or_default() += 1;
        }
        let track_by_path = self
            .tracks
            .iter()
            .map(|track| (track.path.clone(), track))
            .collect::<HashMap<_, _>>();
        let mut top_tracks = self
            .tracks
            .iter()
            .enumerate()
            .filter_map(|(index, track)| {
                play_counts
                    .get(&track.path)
                    .copied()
                    .map(|plays| (plays, index, track.clone()))
            })
            .collect::<Vec<_>>();
        top_tracks.sort_by(|left, right| right.0.cmp(&left.0));
        let mut artist_counts = HashMap::<String, usize>::new();
        for (path, count) in &play_counts {
            if let Some(track) = track_by_path.get(path) {
                for artist in artist_names(&track.artist) {
                    *artist_counts.entry(artist).or_default() += *count;
                }
            }
        }
        let mut top_artists = artist_counts.into_iter().collect::<Vec<_>>();
        top_artists.sort_by(|left, right| right.1.cmp(&left.1));

        ui.heading(RichText::new("Listening stats").size(28.0).strong());
        ui.label(
            RichText::new("A private summary built from listening history stored on this device.")
                .color(Color32::GRAY),
        );
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            for (title, value) in [
                ("Recorded plays", self.history.len()),
                ("Songs played", play_counts.len()),
                ("Liked songs", self.liked_songs.len()),
                ("Artists followed", self.liked_artists.len()),
            ] {
                egui::Frame::new()
                    .fill(Color32::from_rgb(18, 20, 28))
                    .corner_radius(egui::CornerRadius::same(10))
                    .inner_margin(egui::Margin::symmetric(18, 12))
                    .show(ui, |ui| {
                        ui.label(RichText::new(value.to_string()).size(24.0).strong());
                        ui.label(RichText::new(title).small().color(Color32::GRAY));
                    });
            }
        });
        ui.add_space(18.0);
        let mut play_track = None;
        ui.columns(2, |columns| {
            columns[0].heading("Most played songs");
            for (plays, index, track) in top_tracks.iter().take(10) {
                let response = columns[0]
                    .selectable_label(false, format!("{}  ·  {} plays", track.title, plays));
                if response.clicked() {
                    play_track = Some(*index);
                }
                self.show_track_context_menu(&response, *index);
            }
            columns[1].heading("Top artists");
            for (artist, plays) in top_artists.iter().take(10) {
                columns[1].label(format!("{artist}  ·  {plays} plays"));
            }
        });
        if top_tracks.is_empty() {
            ui.add_space(12.0);
            ui.label("Your play history will build this page as you listen.");
        }
        if let Some(index) = play_track {
            self.play_indices(vec![index], 0);
        }
    }

    fn show_library(&mut self, ui: &mut egui::Ui) {
        let page_fade = if self.animations_enabled {
            ui.ctx().animate_bool_with_time(
                egui::Id::new(("page-fade", self.page as u8)),
                true,
                0.22,
            )
        } else {
            1.0
        };
        ui.set_opacity(page_fade);
        let mut indices = if self.page == Page::Songs {
            self.recommendation_indices()
        } else {
            self.page_track_indices()
        };
        if self.page == Page::Songs {
            let query = self.query.trim().to_lowercase();
            indices.retain(|&index| {
                self.tracks.get(index).is_some_and(|track| {
                    !self.excluded_tracks.contains(&track.path)
                        && !self.disliked_songs.contains(&track.path)
                        && (query.is_empty() || track.search_text.contains(&query))
                })
            });
        }
        let title = match self.page {
            Page::Home => "Home".to_string(),
            Page::Discover => "Discover".to_string(),
            Page::Songs => "Quick Picks Mix".to_string(),
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
            Page::Mixes => "Smart mixes".to_string(),
            Page::Stats => "Listening stats".to_string(),
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
                    if !matches!(
                        self.page,
                        Page::Settings | Page::Mixes | Page::Stats | Page::Lyrics | Page::Discover
                    ) {
                        self.show_view_mode_selector(ui);
                    }
                    if self.page == Page::History && ui.small_button("Clear history").clicked() {
                        self.history.clear();
                        self.home_cache_dirty = true;
                        self.save_state();
                    }
                    if !matches!(
                        self.page,
                        Page::Settings | Page::Mixes | Page::Stats | Page::Lyrics
                    ) && self.page != Page::Playlists
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
            Page::Mixes => {
                egui::ScrollArea::vertical()
                    .id_salt("smart-mixes-page")
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.show_smart_mixes(ui));
            }
            Page::Stats => self.show_listening_stats(ui),
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
        let cover = if is_artist {
            let selected_artist = self.selected_artist.clone();
            selected_artist
                .as_deref()
                .and_then(|artist| self.artist_artwork_texture(ui.ctx(), artist))
                .or_else(|| {
                    tracks
                        .first()
                        .and_then(|&index| self.artwork_texture(ui.ctx(), index))
                })
        } else {
            tracks
                .first()
                .and_then(|&index| self.artwork_texture(ui.ctx(), index))
        };
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
                        if icon_text_button(ui, self.ui_icons.get("album"), album.as_str(), false)
                            .clicked()
                        {
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
                        if icon_text_button(ui, self.ui_icons.get("person"), &artist, false)
                            .clicked()
                        {
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
            track_engagement: self.track_engagement.clone(),
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
        let mut clear_filters = false;
        let mut new_to_you_tracks = home
            .unplayed_cards
            .first()
            .map(|card| card.tracks.clone())
            .unwrap_or_default();
        new_to_you_tracks.retain(|&index| {
            self.tracks.get(index).is_some_and(|track| {
                (self.discover_genre.is_empty()
                    || track.genre.eq_ignore_ascii_case(&self.discover_genre))
                    && self.discover_decade.map_or(true, |decade| {
                        track.year >= decade && track.year < decade.saturating_add(10)
                    })
            })
        });
        let fresh_cards = new_to_you_tracks
            .iter()
            .enumerate()
            .filter_map(|(start, &index)| {
                let track = self.tracks.get(index)?;
                Some(HomeCard {
                    title: track.title.clone(),
                    subtitle: display_credit(&track.artist),
                    cover_index: index,
                    tracks: new_to_you_tracks.clone(),
                    start,
                })
            })
            .collect::<Vec<_>>();
        let feature_card = fresh_cards.first().map(|card| HomeCard {
            title: card.title.clone(),
            subtitle: card.subtitle.clone(),
            cover_index: card.cover_index,
            tracks: card.tracks.clone(),
            start: card.start,
        });
        let decades = home
            .decade_mix_cards
            .iter()
            .filter_map(|card| {
                card.tracks
                    .first()
                    .and_then(|&index| self.tracks.get(index))
                    .map(|track| (track.year / 10) * 10)
            })
            .filter(|decade| *decade >= 1950)
            .collect::<BTreeSet<_>>();
        let radio_cards = home
            .mixes
            .iter()
            .filter(|card| card.title.starts_with("Radio:") || card.title.ends_with(" Radio"))
            .map(|card| HomeCard {
                title: card.title.clone(),
                subtitle: card.subtitle.clone(),
                cover_index: card.cover_index,
                tracks: card.tracks.clone(),
                start: card.start,
            })
            .collect::<Vec<_>>();

        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.heading(RichText::new("Explore your library").size(24.0).strong());
                ui.label(
                    RichText::new("Find a new sound, shaped by what you already love.")
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
        ui.add_space(12.0);
        let current_familiarity = if self.discovery < 0.34 {
            "Familiar"
        } else if self.discovery > 0.66 {
            "New"
        } else {
            "Balanced"
        };
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("discover-genre-filter")
                .selected_text(if self.discover_genre.is_empty() {
                    "All genres"
                } else {
                    &self.discover_genre
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.discover_genre, String::new(), "All genres");
                    for (genre, _) in home.genres.iter().take(18) {
                        ui.selectable_value(&mut self.discover_genre, genre.clone(), genre);
                    }
                });
            egui::ComboBox::from_id_salt("discover-era-filter")
                .selected_text(
                    self.discover_decade
                        .map(|decade| format!("{decade}s"))
                        .unwrap_or_else(|| "Any era".to_string()),
                )
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.discover_decade, None, "Any era");
                    for decade in &decades {
                        ui.selectable_value(
                            &mut self.discover_decade,
                            Some(*decade),
                            format!("{decade}s"),
                        );
                    }
                });
            ui.add_space(12.0);
            ui.label(
                RichText::new("FAMILIARITY")
                    .small()
                    .strong()
                    .color(Color32::from_rgb(170, 174, 212)),
            );
            for (label, value) in [("Familiar", 0.15_f32), ("Balanced", 0.5), ("New", 0.85)] {
                if ui
                    .selectable_label(
                        current_familiarity == label,
                        RichText::new(label).size(13.0),
                    )
                    .clicked()
                    && (self.discovery - value).abs() > f32::EPSILON
                {
                    self.discovery = value;
                    self.save_state();
                    refresh = true;
                }
            }
        });
        ui.add_space(12.0);
        if self.tracks.is_empty() {
            ui.label("Add a music folder to build personalized mixes.");
        }
        egui::ScrollArea::vertical()
            .id_salt("discover-feed")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.label(
                    RichText::new("YOUR NEXT DISCOVERY")
                        .small()
                        .strong()
                        .color(Color32::from_rgb(170, 174, 212)),
                );
                ui.heading(RichText::new("Fresh picks").size(26.0).strong());
                if let Some(feature_card) = feature_card.as_ref() {
                    self.show_home_feature(
                        ui,
                        Some(feature_card),
                        "A FRESH PICK FOR YOUR LIBRARY",
                        "MORE TO EXPLORE",
                    );
                    ui.add_space(18.0);
                    self.show_home_shelf(
                        ui,
                        "New to you",
                        "RECOMMENDED TRACKS YOU HAVEN’T PLAYED YET",
                        &fresh_cards,
                        None,
                    );
                } else {
                    ui.add_space(8.0);
                    egui::Frame::new()
                        .fill(Color32::from_rgb(23, 24, 42))
                        .corner_radius(egui::CornerRadius::same(16))
                        .inner_margin(egui::Margin::same(18))
                        .show(ui, |ui| {
                            ui.label("No new picks match these filters.");
                            if ui.button("Clear filters").clicked() {
                                clear_filters = true;
                            }
                        });
                }
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
                    "Artist radio",
                    "MORE FROM ARTISTS YOU LIKE",
                    &radio_cards,
                    None,
                );
            });
        if clear_filters {
            self.discover_genre.clear();
            self.discover_decade = None;
        }
        self.home_cache = Some(home);
        if refresh {
            self.recommendations_dirty = true;
            self.home_cache_dirty = true;
            self.mix_refresh_requested = true;
            self.drive_home_jobs(ui.ctx());
        }
    }

    fn show_home_feature(
        &mut self,
        ui: &mut egui::Ui,
        card: Option<&HomeCard>,
        feature_label: &str,
        preview_label: &str,
    ) {
        let Some(card) = card.filter(|card| !card.tracks.is_empty()) else {
            egui::Frame::new()
                .fill(Color32::from_rgb(24, 25, 44))
                .corner_radius(egui::CornerRadius::same(18))
                .inner_margin(egui::Margin::same(20))
                .show(ui, |ui| {
                    ui.heading("Your listening starts here");
                    ui.label(
                        RichText::new("Add music to your library and Lilt will build a personal mix for you.")
                            .color(Color32::from_rgb(180, 180, 194)),
                    );
                });
            return;
        };

        let first_position = card.start.min(card.tracks.len() - 1);
        let preview = (0..card.tracks.len().min(4))
            .filter_map(|offset| {
                let position = (first_position + offset) % card.tracks.len();
                let index = *card.tracks.get(position)?;
                let track = self.tracks.get(index)?;
                Some((
                    position,
                    index,
                    track.title.clone(),
                    display_credit(&track.artist),
                    display_metadata(&track.album),
                ))
            })
            .collect::<Vec<_>>();
        let Some((_, featured_index, featured_title, featured_artist, featured_album)) =
            preview.first().cloned()
        else {
            return;
        };
        let featured_cover = self.artwork_texture(ui.ctx(), featured_index);
        let mut play_position = None;

        egui::Frame::new()
            .fill(Color32::from_rgb(23, 24, 42))
            .stroke(egui::Stroke::new(
                1.0_f32,
                accent_color(&self.accent).linear_multiply(0.32),
            ))
            .corner_radius(egui::CornerRadius::same(18))
            .inner_margin(egui::Margin::same(16))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.columns(2, |columns| {
                    let left = &mut columns[0];
                    left.horizontal(|ui| {
                        let (cover_rect, cover_response) = ui.allocate_exact_size(
                            Vec2::splat(184.0),
                            egui::Sense::click(),
                        );
                        if let Some(cover) = &featured_cover {
                            ui.painter().image(
                                cover.id(),
                                cover_rect,
                                egui::Rect::from_min_max(
                                    egui::Pos2::ZERO,
                                    egui::pos2(1.0, 1.0),
                                ),
                                Color32::WHITE,
                            );
                        } else {
                            let colors = [
                                Color32::from_rgb(48, 66, 132),
                                Color32::from_rgb(76, 54, 132),
                                Color32::from_rgb(35, 105, 112),
                                Color32::from_rgb(113, 79, 139),
                            ];
                            ui.painter().rect_filled(
                                cover_rect,
                                egui::CornerRadius::same(12),
                                colors[featured_index % colors.len()],
                            );
                            ui.painter().text(
                                cover_rect.center(),
                                egui::Align2::CENTER_CENTER,
                                "♫",
                                egui::FontId::proportional(58.0),
                                Color32::WHITE,
                            );
                        }
                        ui.painter().rect_stroke(
                            cover_rect,
                            egui::CornerRadius::same(12),
                            egui::Stroke::new(1.0_f32, Color32::from_white_alpha(28)),
                            egui::StrokeKind::Inside,
                        );
                        if cover_response.clicked() {
                            play_position = Some(first_position);
                        }
                        cover_response.on_hover_text("Play this selection");

                        ui.vertical(|ui| {
                            ui.label(
                                RichText::new(feature_label)
                                .small()
                                .strong()
                                .color(Color32::from_rgb(182, 184, 223)),
                            );
                            ui.add_space(10.0);
                            ui.add_sized(
                                [ui.available_width(), 58.0],
                                egui::Label::new(
                                    RichText::new(&featured_title).size(25.0).strong(),
                                )
                                .truncate(),
                            );
                            ui.add_sized(
                                [ui.available_width(), 36.0],
                                egui::Label::new(
                                    RichText::new(format!("{featured_artist} · {featured_album}"))
                                        .color(Color32::from_rgb(187, 189, 201)),
                                )
                                .truncate(),
                            );
                            ui.add_space(10.0);
                            if ui
                                .add_sized(
                                    [132.0, 38.0],
                                    egui::Button::new("▶  Play now")
                                        .fill(accent_color(&self.accent)),
                                )
                                .clicked()
                            {
                                play_position = Some(first_position);
                            }
                        });
                    });

                    let right = &mut columns[1];
                    right.vertical(|ui| {
                        ui.label(
                            RichText::new(preview_label)
                            .small()
                            .strong()
                            .color(Color32::from_rgb(182, 184, 223)),
                        );
                        ui.add_space(8.0);
                        for (position, index, title, artist, _) in &preview {
                            let cover = self.artwork_texture(ui.ctx(), *index);
                            let (row_rect, response) = ui.allocate_exact_size(
                                Vec2::new(ui.available_width(), 43.0),
                                egui::Sense::click(),
                            );
                            ui.painter().rect_filled(
                                row_rect,
                                egui::CornerRadius::same(9),
                                if response.hovered() {
                                    Color32::from_rgb(43, 44, 66)
                                } else {
                                    Color32::from_rgb(31, 32, 51)
                                },
                            );
                            let inner = row_rect.shrink2(Vec2::new(6.0, 4.0));
                            let thumb_rect = egui::Rect::from_min_size(
                                inner.min,
                                Vec2::splat(35.0),
                            );
                            if let Some(cover) = cover {
                                ui.painter().image(
                                    cover.id(),
                                    thumb_rect,
                                    egui::Rect::from_min_max(
                                        egui::Pos2::ZERO,
                                        egui::pos2(1.0, 1.0),
                                    ),
                                    Color32::WHITE,
                                );
                            } else {
                                ui.painter().rect_filled(
                                    thumb_rect,
                                    egui::CornerRadius::same(6),
                                    Color32::from_rgb(51, 57, 92),
                                );
                            }
                            let text_rect = egui::Rect::from_min_max(
                                egui::pos2(thumb_rect.max.x + 10.0, inner.min.y),
                                egui::pos2(inner.max.x - 2.0, inner.max.y),
                            );
                            ui.painter().text(
                                egui::pos2(text_rect.min.x, text_rect.center().y - 7.0),
                                egui::Align2::LEFT_CENTER,
                                title,
                                egui::FontId::proportional(14.0),
                                Color32::from_rgb(239, 239, 244),
                            );
                            ui.painter().text(
                                egui::pos2(text_rect.min.x, text_rect.center().y + 9.0),
                                egui::Align2::LEFT_CENTER,
                                artist,
                                egui::FontId::proportional(11.0),
                                Color32::from_rgb(169, 171, 188),
                            );
                            if response.clicked() {
                                play_position = Some(*position);
                            }
                            response.on_hover_text(format!("Play {title}"));
                        }
                    });
                });
            });

        if let Some(position) = play_position {
            self.play_indices(card.tracks.clone(), position);
        }
    }

    fn show_home_quick_picks(&mut self, ui: &mut egui::Ui) {
        let mut pool = self.recommendations.clone();
        pool.retain(|&index| {
            self.tracks.get(index).is_some_and(|track| {
                !self.excluded_tracks.contains(&track.path)
                    && !self.disliked_songs.contains(&track.path)
                    && (self.home_genre.is_empty()
                        || track.genre.eq_ignore_ascii_case(&self.home_genre))
            })
        });
        if pool.is_empty() {
            pool = self
                .tracks
                .iter()
                .enumerate()
                .filter(|(_, track)| {
                    !self.excluded_tracks.contains(&track.path)
                        && !self.disliked_songs.contains(&track.path)
                        && (self.home_genre.is_empty()
                            || track.genre.eq_ignore_ascii_case(&self.home_genre))
                })
                .map(|(index, _)| index)
                .collect();
        }
        let pages = pool.len().div_ceil(12).max(1);
        self.quick_pick_page %= pages;
        let page_start = self.quick_pick_page * 12;
        let picks = pool
            .iter()
            .skip(page_start)
            .take(12)
            .copied()
            .collect::<Vec<_>>();
        let mut play_all = false;
        let mut play_index = None;

        egui::Frame::new()
            .fill(Color32::from_rgb(8, 9, 13))
            .stroke(egui::Stroke::new(1.0_f32, Color32::from_rgb(31, 34, 45)))
            .corner_radius(egui::CornerRadius::same(12))
            .inner_margin(egui::Margin::symmetric(14, 12))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.heading(RichText::new("Quick picks").size(23.0).strong());
                    ui.label(
                        RichText::new(if self.home_genre.is_empty() {
                            "A few good places to start"
                        } else {
                            "Picked from this genre"
                        })
                        .small()
                        .color(Color32::GRAY),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_enabled(!picks.is_empty(), egui::Button::new("Play all"))
                            .clicked()
                        {
                            play_all = true;
                        }
                        if ui
                            .add_enabled(pages > 1, egui::Button::new("›"))
                            .on_hover_text("More quick picks")
                            .clicked()
                        {
                            self.quick_pick_page = (self.quick_pick_page + 1) % pages;
                        }
                        ui.label(
                            RichText::new(format!("{} / {}", self.quick_pick_page + 1, pages))
                                .small()
                                .color(Color32::GRAY),
                        );
                        if ui
                            .add_enabled(pages > 1, egui::Button::new("‹"))
                            .on_hover_text("Previous quick picks")
                            .clicked()
                        {
                            self.quick_pick_page = (self.quick_pick_page + pages - 1) % pages;
                        }
                    });
                });
                ui.add_space(8.0);
                if picks.is_empty() {
                    ui.label(
                        RichText::new("Add music to your library to get quick picks.")
                            .color(Color32::GRAY),
                    );
                } else {
                    ui.columns(3, |columns| {
                        for (column_index, column) in columns.iter_mut().enumerate() {
                            let start = column_index * 4;
                            for (row, &track_index) in picks.iter().skip(start).take(4).enumerate()
                            {
                                let Some((title, artist, album)) =
                                    self.tracks.get(track_index).map(|track| {
                                        (
                                            track.title.clone(),
                                            display_credit(&track.artist),
                                            display_metadata(&track.album),
                                        )
                                    })
                                else {
                                    continue;
                                };
                                let cover = self.artwork_texture(column.ctx(), track_index);
                                let (row_rect, response) = column.allocate_exact_size(
                                    Vec2::new(column.available_width(), 60.0),
                                    egui::Sense::click(),
                                );
                                let response = response
                                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                                    .on_hover_text(
                                        "Play this quick pick · right-click for more actions",
                                    );
                                let row_fill = if response.hovered() {
                                    Color32::from_rgb(8, 9, 13)
                                } else {
                                    Color32::from_rgb(13, 14, 19)
                                };
                                column.painter().rect_filled(
                                    row_rect,
                                    egui::CornerRadius::same(8),
                                    row_fill,
                                );
                                column.scope_builder(
                                    egui::UiBuilder::new()
                                        .max_rect(row_rect.shrink2(Vec2::new(5.0, 4.0)))
                                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                                    |ui| {
                                        ui.horizontal(|ui| {
                                            if let Some(cover) = &cover {
                                                ui.add(
                                                    egui::Image::new((
                                                        cover.id(),
                                                        Vec2::splat(48.0),
                                                    ))
                                                    .corner_radius(egui::CornerRadius::same(12)),
                                                );
                                            } else {
                                                let (rect, _) = ui.allocate_exact_size(
                                                    Vec2::splat(48.0),
                                                    egui::Sense::hover(),
                                                );
                                                paint_cover_or_placeholder(
                                                    ui,
                                                    None,
                                                    rect,
                                                    track_index,
                                                );
                                            }
                                            let text_width = (ui.available_width() - 4.0).max(40.0);
                                            ui.vertical(|ui| {
                                                ui.add_sized(
                                                    [text_width, 23.0],
                                                    egui::Label::new(
                                                        RichText::new(&title).strong(),
                                                    )
                                                    .truncate(),
                                                );
                                                ui.add_sized(
                                                    [text_width, 19.0],
                                                    egui::Label::new(
                                                        RichText::new(format!(
                                                            "{artist} · {album}"
                                                        ))
                                                        .small()
                                                        .color(Color32::from_rgb(178, 182, 194)),
                                                    )
                                                    .truncate(),
                                                );
                                            });
                                        });
                                    },
                                );
                                let clicked = response.clicked();
                                if clicked {
                                    play_index = Some(track_index);
                                }
                                self.show_track_context_menu(&response, track_index);
                                if row < 3 {
                                    column.add_space(3.0);
                                }
                            }
                        }
                    });
                }
            });

        if play_all && !picks.is_empty() {
            self.play_indices(picks, 0);
        } else if let Some(index) = play_index {
            let start = picks.iter().position(|&pick| pick == index).unwrap_or(0);
            self.play_indices(picks, start);
        }
    }

    fn show_home(&mut self, ui: &mut egui::Ui) {
        if self.search_committed && !self.query.trim().is_empty() {
            self.show_home_search_results(ui);
            return;
        }
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
                            RichText::new("YOUR LISTENING SPACE")
                                .small()
                                .strong()
                                .color(Color32::from_rgb(164, 167, 224)),
                        );
                        ui.label(
                            RichText::new("Made for the way you listen")
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
                let feature_card = if home.welcome_resume.is_some() {
                    home.recent_cards.first()
                } else {
                    home.unplayed_cards
                        .first()
                        .or_else(|| home.daily_mix_cards.first())
                        .or_else(|| home.recent_cards.first())
                };
                self.show_home_feature(
                    ui,
                    feature_card,
                    if home.welcome_resume.is_some() {
                        "PICK UP WHERE YOU LEFT OFF"
                    } else {
                        "A PLACE TO START"
                    },
                    if home.welcome_resume.is_some() {
                        "IN YOUR RECENT ROTATION"
                    } else {
                        "UP NEXT FOR YOU"
                    },
                );
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
                self.show_home_quick_picks(ui);
                ui.add_space(16.0);
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
                    "From your library",
                    "ALBUMS READY FOR ANOTHER LISTEN",
                    &home.library_cards,
                    Some(Page::Albums),
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
                    "Recently added",
                    "NEW IN YOUR COLLECTION",
                    &home.latest_cards,
                    Some(Page::Songs),
                );
            });
        self.home_cache = Some(home);
    }

    fn search_matches(&self, query: &str) -> Vec<usize> {
        let query = query.trim().to_lowercase();
        let terms = query
            .split_whitespace()
            .filter(|term| term.len() > 1)
            .collect::<Vec<_>>();
        let mut matches = self
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, track)| {
                !self.excluded_tracks.contains(&track.path)
                    && !self.disliked_songs.contains(&track.path)
                    && (track.search_text.contains(&query)
                        || (!terms.is_empty()
                            && terms.iter().all(|term| track.search_text.contains(term))))
            })
            .map(|(index, track)| {
                let title = track.title.to_lowercase();
                let artist = track.artist.to_lowercase();
                let album = track.album.to_lowercase();
                let all_in = |value: &str| {
                    !terms.is_empty() && terms.iter().all(|term| value.contains(term))
                };
                let rank = if title.starts_with(&query) {
                    0
                } else if title.contains(&query) {
                    1
                } else if artist.starts_with(&query) || all_in(&artist) {
                    2
                } else if album.starts_with(&query) || all_in(&album) {
                    3
                } else if all_in(&title) {
                    4
                } else {
                    5
                };
                (index, rank, title)
            })
            .collect::<Vec<_>>();
        matches.sort_by(|left, right| left.1.cmp(&right.1).then_with(|| left.2.cmp(&right.2)));
        matches.into_iter().map(|(index, _, _)| index).collect()
    }

    fn remember_search(&mut self, query: &str) {
        let query = query.trim();
        if query.is_empty() {
            return;
        }
        self.search_history
            .retain(|recent| !recent.eq_ignore_ascii_case(query));
        self.search_history.insert(0, query.to_owned());
        self.search_history.truncate(5);
        self.save_state();
    }

    fn show_home_search_results(&mut self, ui: &mut egui::Ui) {
        let query = self.query.trim().to_owned();
        let matches = self.search_matches(&query);
        ui.heading(format!("Search results for ‘{query}’"));
        ui.label(
            RichText::new("Songs, artists, and albums from your library").color(Color32::GRAY),
        );
        ui.add_space(10.0);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let mut play = None;
                ui.heading("Songs");
                if matches.is_empty() {
                    ui.label("No matching songs in your library.");
                }
                for (row, &index) in matches.iter().take(40).enumerate() {
                    let track = &self.tracks[index];
                    if ui
                        .selectable_label(
                            false,
                            format!("{}  ·  {}  ·  {}", track.title, track.artist, track.album),
                        )
                        .clicked()
                    {
                        play = Some((index, matches.clone()));
                    }
                    if row == 39 {
                        ui.label("Showing the first 40 matches.");
                    }
                }
                if let Some((index, queue)) = play {
                    let start = queue.iter().position(|&item| item == index).unwrap_or(0);
                    self.play_indices(queue, start);
                }

                ui.add_space(12.0);
                ui.heading("Artists");
                let mut artists = self
                    .tracks
                    .iter()
                    .enumerate()
                    .filter(|(_, track)| {
                        !self.excluded_tracks.contains(&track.path)
                            && track.artist.to_lowercase().contains(&query)
                    })
                    .map(|(_, track)| track.artist.trim().to_owned())
                    .filter(|artist| !artist.is_empty())
                    .collect::<Vec<_>>();
                artists.sort_by_key(|artist| artist.to_lowercase());
                artists.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
                let mut artist_open = None;
                for artist in artists.iter().take(12) {
                    if ui.button(format!("♫  {artist}")).clicked() {
                        artist_open = Some(artist.clone());
                    }
                }
                if let Some(artist) = artist_open {
                    self.selected_artist = Some(artist);
                    self.page = Page::ArtistDetail;
                }

                ui.add_space(12.0);
                ui.heading("Albums");
                let mut albums = self
                    .tracks
                    .iter()
                    .enumerate()
                    .filter(|(_, track)| {
                        !self.excluded_tracks.contains(&track.path)
                            && track.album.to_lowercase().contains(&query)
                    })
                    .map(|(_, track)| track.album.trim().to_owned())
                    .filter(|album| !album.is_empty())
                    .collect::<Vec<_>>();
                albums.sort_by_key(|album| album.to_lowercase());
                albums.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
                let mut album_open = None;
                for album in albums.iter().take(12) {
                    if icon_text_button(ui, self.ui_icons.get("album"), album.as_str(), false)
                        .clicked()
                    {
                        album_open = Some(album.clone());
                    }
                }
                if let Some(album) = album_open {
                    self.selected_album = Some(album);
                    self.page = Page::AlbumDetail;
                }

                ui.add_space(12.0);
                ui.heading("Recommendations");
                let recommended = if !self.search_ai_results.is_empty() {
                    self.search_ai_results.clone()
                } else {
                    self.recommendation_indices()
                        .into_iter()
                        .filter(|&index| {
                            self.tracks
                                .get(index)
                                .is_some_and(|track| track.search_text.contains(&query))
                        })
                        .take(5)
                        .collect::<Vec<_>>()
                };
                if recommended.is_empty() {
                    ui.label(
                        RichText::new("No additional recommendations for this search yet.")
                            .color(Color32::GRAY),
                    );
                }
                for &index in &recommended {
                    let track = &self.tracks[index];
                    if ui
                        .button(format!("▶  {}  ·  {}", track.title, track.artist))
                        .clicked()
                    {
                        self.play_indices(
                            recommended.clone(),
                            recommended
                                .iter()
                                .position(|&item| item == index)
                                .unwrap_or(0),
                        );
                    }
                }
            });
    }

    fn show_search_suggestions(&mut self, ctx: &egui::Context, search_rect: egui::Rect) {
        let query = self.query.trim().to_owned();
        let history_view = query.is_empty();
        if history_view && self.search_history.is_empty() {
            self.search_suggestions_open = false;
            return;
        }
        let query_terms = query
            .to_lowercase()
            .split_whitespace()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let should_open =
            self.search_suggestions_open && (!history_view || !self.search_history.is_empty());
        let openness = if self.animations_enabled {
            ctx.animate_bool_with_time_and_easing(
                egui::Id::new("live-search-suggestions-motion"),
                should_open,
                0.17,
                egui::emath::easing::quadratic_out,
            )
        } else if should_open {
            1.0
        } else {
            0.0
        };
        if openness <= 0.01 {
            return;
        }
        let matches = if history_view {
            Vec::new()
        } else {
            self.search_matches(&query)
        };
        let songs = matches.iter().copied().take(5).collect::<Vec<_>>();
        let mut artists = self
            .tracks
            .iter()
            .filter(|track| {
                !query.is_empty()
                    && !self.excluded_tracks.contains(&track.path)
                    && (track.artist.to_lowercase().contains(&query.to_lowercase())
                        || query_terms
                            .iter()
                            .all(|term| track.artist.to_lowercase().contains(term)))
            })
            .map(|track| track.artist.trim().to_owned())
            .filter(|artist| !artist.is_empty())
            .collect::<Vec<_>>();
        artists.sort_by_key(|artist| {
            (
                !artist.to_lowercase().starts_with(&query.to_lowercase()),
                artist.to_lowercase(),
            )
        });
        artists.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
        artists.truncate(3);
        let mut albums = self
            .tracks
            .iter()
            .filter(|track| {
                !query.is_empty()
                    && !self.excluded_tracks.contains(&track.path)
                    && (track.album.to_lowercase().contains(&query.to_lowercase())
                        || query_terms
                            .iter()
                            .all(|term| track.album.to_lowercase().contains(term)))
            })
            .map(|track| track.album.trim().to_owned())
            .filter(|album| !album.is_empty())
            .collect::<Vec<_>>();
        albums.sort_by_key(|album| {
            (
                !album.to_lowercase().starts_with(&query.to_lowercase()),
                album.to_lowercase(),
            )
        });
        albums.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
        albums.truncate(3);
        let artist_first = artists.first().is_some_and(|artist| {
            let artist = artist.to_lowercase();
            artist.starts_with(&query.to_lowercase())
                || query_terms.iter().all(|term| artist.contains(term))
        });
        let mut suggestions = Vec::new();
        if artist_first {
            if let Some(artist) = artists.first() {
                suggestions.push(SearchSuggestion::Artist(artist.clone()));
            }
            for &index in &songs {
                if suggestions.len() >= 4 {
                    break;
                }
                if self.tracks.get(index).is_some_and(|track| {
                    artist_names(&track.artist)
                        .iter()
                        .any(|name| name.eq_ignore_ascii_case(artists[0].as_str()))
                }) {
                    suggestions.push(SearchSuggestion::Song(index));
                }
            }
        } else {
            suggestions.extend(songs.iter().take(3).copied().map(SearchSuggestion::Song));
            if let Some(artist) = artists.first() {
                suggestions.push(SearchSuggestion::Artist(artist.clone()));
            }
        }
        if let Some(album) = albums.first() {
            if let Some(index) = self
                .tracks
                .iter()
                .position(|track| {
                    track.album.eq_ignore_ascii_case(album) && track.artwork.is_some()
                })
                .or_else(|| {
                    self.tracks
                        .iter()
                        .position(|track| track.album.eq_ignore_ascii_case(album))
                })
            {
                if suggestions.len() < 5 {
                    suggestions.push(SearchSuggestion::Album(album.clone(), index));
                }
            }
        }
        for &index in &songs {
            if suggestions.len() >= 5 {
                break;
            }
            if !suggestions.iter().any(|suggestion| matches!(suggestion, SearchSuggestion::Song(existing) if *existing == index)) {
                suggestions.push(SearchSuggestion::Song(index));
            }
        }
        for artist in artists.iter().skip(1) {
            if suggestions.len() >= 5 {
                break;
            }
            suggestions.push(SearchSuggestion::Artist(artist.clone()));
        }
        for album in albums.iter().skip(1) {
            if suggestions.len() >= 5 {
                break;
            }
            if let Some(index) = self
                .tracks
                .iter()
                .position(|track| track.album.eq_ignore_ascii_case(album))
            {
                suggestions.push(SearchSuggestion::Album(album.clone(), index));
            }
        }
        let keyboard = ctx.input(|input| {
            (
                input.key_pressed(egui::Key::ArrowDown),
                input.key_pressed(egui::Key::ArrowUp),
                input.key_pressed(egui::Key::Enter),
                input.key_pressed(egui::Key::Escape),
                input.pointer.any_pressed(),
                input.pointer.interact_pos(),
            )
        });
        let search_field_focused =
            ctx.memory(|memory| memory.focused() == Some(egui::Id::new("global-search")));
        if keyboard.3 {
            self.search_suggestions_open = false;
            self.search_suggestions_focused = false;
            return;
        }
        if keyboard.4 {
            if let Some(position) = keyboard
                .5
                .filter(|position| !search_rect.contains(*position))
            {
                // Keep the dropdown open while interacting with its own rows.
                let hovering_popup = ctx
                    .layer_id_at(position)
                    .is_some_and(|layer| layer.id == egui::Id::new("live-search-suggestions"));
                if !hovering_popup {
                    self.search_suggestions_open = false;
                    self.search_suggestions_focused = false;
                }
            }
        }
        let ai_row = !history_view && self.google_ai_key_saved;
        let navigable_rows = if history_view {
            self.search_history.len()
        } else {
            suggestions.len() + usize::from(ai_row)
        };
        if keyboard.0 {
            if search_field_focused {
                self.search_suggestions_focused = true;
                self.search_suggestion_selection = 0;
            } else if navigable_rows > 0 {
                self.search_suggestion_selection =
                    (self.search_suggestion_selection + 1) % navigable_rows;
            }
        }
        if keyboard.1 {
            if self.search_suggestions_focused && self.search_suggestion_selection == 0 {
                self.search_suggestions_focused = false;
                ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("global-search")));
            } else if navigable_rows > 0 {
                self.search_suggestion_selection =
                    self.search_suggestion_selection.saturating_sub(1);
            }
        }
        let mut play = None;
        let mut artist_open = None;
        let mut album_open = None;
        let mut ask_ai = false;
        let mut history_open: Option<String> = None;
        let mut history_delete: Option<String> = None;
        if keyboard.2 && self.search_suggestions_focused {
            if history_view {
                history_open = self
                    .search_history
                    .get(self.search_suggestion_selection)
                    .cloned();
            } else if ai_row && self.search_suggestion_selection == 0 {
                ask_ai = !self.search_ai_pending;
            } else if let Some(suggestion) = suggestions.get(
                self.search_suggestion_selection
                    .saturating_sub(usize::from(ai_row)),
            ) {
                match suggestion {
                    SearchSuggestion::Song(index) => play = Some((*index, songs.clone())),
                    SearchSuggestion::Artist(artist) => artist_open = Some(artist.clone()),
                    SearchSuggestion::Album(album, _) => album_open = Some(album.clone()),
                }
            }
        }
        // A single scroll-area ID keeps its offset as search results change. If a long
        // result list was scrolled and the next query is shorter, that stale offset
        // can leave the popup looking clipped; when the list grows again it can seem
        // like the popup never expanded. Reset only when the query changes so normal
        // scrolling remains available while the user browses one query.
        let scroll_reset_id = egui::Id::new("live-search-suggestions-query");
        let query_changed = ctx.data_mut(|data| {
            let previous = data.get_temp::<String>(scroll_reset_id);
            let changed = previous.as_deref() != Some(query.as_str());
            data.insert_temp(scroll_reset_id, query.clone());
            changed
        });
        let screen = ctx.screen_rect();
        let space_below = (screen.bottom() - search_rect.bottom() - 16.0).max(0.0);
        let space_above = (search_rect.top() - screen.top() - 16.0).max(0.0);
        let preferred_height = if history_view {
            (self.search_history.len() as f32 * 62.0 + 24.0).clamp(120.0, 420.0)
        } else {
            (screen.height() * 0.68).clamp(360.0, 720.0)
        };
        let popup_above = space_below < preferred_height + 4.0 && space_above > space_below;
        // Use the largest practical canvas for this window. Its height now changes
        // with the app size, not with each query's result count.
        let available_height = ((if popup_above {
            space_above
        } else {
            space_below
        }) - 76.0)
            .min(preferred_height)
            .max(1.0);
        let popup_width = search_rect
            .width()
            .max(620.0)
            .min(ctx.screen_rect().right() - search_rect.left() - 16.0);
        egui::Area::new(egui::Id::new("live-search-suggestions"))
            .order(egui::Order::Foreground)
            .fixed_pos(if popup_above {
                search_rect.left_top() - egui::vec2(0.0, 80.0 + available_height)
                    + egui::vec2(0.0, (1.0 - openness) * 10.0)
            } else {
                search_rect.left_bottom() + egui::vec2(0.0, 4.0 - (1.0 - openness) * 10.0)
            })
            .show(ctx, |ui| {
                ui.set_opacity(openness);
                egui::Frame::popup(ui.style())
                    .fill(Color32::from_rgb(12, 13, 18))
                    .stroke(egui::Stroke::new(1.0_f32, Color32::from_rgb(56, 59, 70)))
                    .inner_margin(egui::Margin::symmetric(12, 10))
                    .corner_radius(egui::CornerRadius::same(12))
                    .show(ui, |ui| {
                        ui.set_min_width(popup_width);
                        ui.set_max_width(popup_width);
                        ui.horizontal(|ui| {
                            if let Some(icon) = self.ui_icons.get("search") {
                                ui.add(egui::Image::new(icon).fit_to_exact_size(Vec2::splat(21.0)));
                            }
                            ui.label(RichText::new("Search").size(19.0).strong());
                        });
                        ui.add_space(6.0);
                        let mut scroll_area = egui::ScrollArea::vertical()
                            .auto_shrink([true, false])
                            .max_height(available_height);
                        if query_changed {
                            scroll_area = scroll_area.vertical_scroll_offset(0.0);
                        }
                        let recent_searches = self.search_history.clone();
                        scroll_area.show(ui, |ui| {
                            if history_view {
                                ui.label(
                                    RichText::new("Recent searches")
                                        .small()
                                        .strong()
                                        .color(Color32::from_rgb(157, 164, 181)),
                                );
                                ui.add_space(4.0);
                                for (row, recent) in recent_searches.iter().enumerate() {
                                    let selected = self.search_suggestions_focused
                                        && self.search_suggestion_selection == row;
                                    ui.horizontal(|ui| {
                                        if let Some(icon) = self.ui_icons.get("history") {
                                            ui.add(
                                                egui::Image::new(icon)
                                                    .fit_to_exact_size(Vec2::splat(20.0)),
                                            );
                                        }
                                        if ui
                                            .selectable_label(
                                                selected,
                                                RichText::new(recent).size(17.0),
                                            )
                                            .clicked()
                                        {
                                            history_open = Some(recent.clone());
                                        }
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                if let Some(icon) = self.ui_icons.get("delete") {
                                                    if ui
                                                        .add(
                                                            egui::Button::image(
                                                                egui::Image::new(icon)
                                                                    .fit_to_exact_size(
                                                                        Vec2::splat(18.0),
                                                                    ),
                                                            )
                                                            .frame(false),
                                                        )
                                                        .on_hover_text("Remove from search history")
                                                        .clicked()
                                                    {
                                                        history_delete = Some(recent.clone());
                                                    }
                                                }
                                            },
                                        );
                                    });
                                    ui.add_space(3.0);
                                }
                            } else {
                                if ai_row {
                                    let selected = self.search_suggestions_focused
                                        && self.search_suggestion_selection == 0;
                                    let mut clicked = false;
                                    ui.horizontal(|ui| {
                                        if let Some(icon) = self.ui_icons.get("sparkles") {
                                            ui.add(
                                                egui::Image::new(icon)
                                                    .fit_to_exact_size(Vec2::splat(48.0)),
                                            );
                                        }
                                        clicked = ui
                                            .add_enabled(
                                                !self.search_ai_pending,
                                                egui::Button::new(
                                                    RichText::new(format!("Ask AI · {query}"))
                                                        .size(18.0)
                                                        .strong(),
                                                )
                                                .selected(selected),
                                            )
                                            .clicked();
                                    });
                                    if clicked {
                                        ask_ai = true;
                                    }
                                    ui.label(
                                        RichText::new("Ask Google AI about your library")
                                            .small()
                                            .color(Color32::from_rgb(182, 185, 196)),
                                    );
                                    ui.separator();
                                }
                                if suggestions.is_empty() {
                                    ui.horizontal(|ui| {
                                        if let Some(icon) = self.ui_icons.get("search") {
                                            ui.add(
                                                egui::Image::new(icon)
                                                    .fit_to_exact_size(Vec2::splat(18.0)),
                                            );
                                        }
                                        ui.label(
                                            RichText::new(
                                                "No matches. Try another title, artist, or album.",
                                            )
                                            .color(Color32::GRAY),
                                        );
                                    });
                                }
                                for (row, suggestion) in suggestions.iter().enumerate() {
                                    let selected = self.search_suggestions_focused
                                        && row + usize::from(ai_row)
                                            == self.search_suggestion_selection;
                                    let clicked = match suggestion {
                                        SearchSuggestion::Song(index) => {
                                            let cover = self.artwork_texture(ui.ctx(), *index);
                                            let (title, artist, album) = self
                                                .tracks
                                                .get(*index)
                                                .map(|track| {
                                                    (
                                                        track.title.clone(),
                                                        track.artist.clone(),
                                                        track.album.clone(),
                                                    )
                                                })
                                                .unwrap_or_default();
                                            let response = ui
                                                .horizontal(|ui| {
                                                    if let Some(texture) = cover {
                                                        ui.add(
                                                            egui::Image::new(&texture)
                                                                .fit_to_exact_size(Vec2::splat(
                                                                    56.0,
                                                                )),
                                                        );
                                                    } else {
                                                        let (rect, _) = ui.allocate_exact_size(
                                                            Vec2::splat(56.0),
                                                            egui::Sense::hover(),
                                                        );
                                                        paint_cover_or_placeholder(
                                                            ui, None, rect, *index,
                                                        );
                                                    }
                                                    ui.vertical(|ui| {
                                                        let title_response = ui.selectable_label(
                                                            selected,
                                                            RichText::new(&title)
                                                                .size(18.0)
                                                                .strong(),
                                                        );
                                                        ui.label(
                                                        RichText::new(format!(
                                                            "Song  ·  {}  ·  {}",
                                                            display_metadata(&artist),
                                                            display_metadata(&album)
                                                        ))
                                                        .size(14.0)
                                                        .color(Color32::from_rgb(182, 185, 196)),
                                                    );
                                                        title_response
                                                    })
                                                    .inner
                                                })
                                                .inner;
                                            if response.clicked() {
                                                play = Some((*index, songs.clone()));
                                            }
                                            false
                                        }
                                        SearchSuggestion::Artist(artist) => {
                                            let mut open = false;
                                            ui.horizontal(|ui| {
                                                if let Some(icon) = self.ui_icons.get("person") {
                                                    ui.add(
                                                        egui::Image::new(icon)
                                                            .fit_to_exact_size(Vec2::splat(48.0)),
                                                    );
                                                }
                                                ui.vertical(|ui| {
                                                    open = ui
                                                        .selectable_label(
                                                            selected,
                                                            RichText::new(artist)
                                                                .size(18.0)
                                                                .strong(),
                                                        )
                                                        .clicked();
                                                    ui.label(
                                                        RichText::new("Artist").size(14.0).color(
                                                            Color32::from_rgb(182, 185, 196),
                                                        ),
                                                    );
                                                });
                                            });
                                            if open {
                                                artist_open = Some(artist.clone());
                                            }
                                            false
                                        }
                                        SearchSuggestion::Album(album, index) => {
                                            let cover = self.artwork_texture(ui.ctx(), *index);
                                            let artist = self
                                                .tracks
                                                .get(*index)
                                                .map(|track| track.artist.clone())
                                                .unwrap_or_default();
                                            let mut open = false;
                                            ui.horizontal(|ui| {
                                                if let Some(texture) = cover {
                                                    ui.add(
                                                        egui::Image::new(&texture)
                                                            .fit_to_exact_size(Vec2::splat(56.0)),
                                                    );
                                                } else {
                                                    let (rect, _) = ui.allocate_exact_size(
                                                        Vec2::splat(56.0),
                                                        egui::Sense::hover(),
                                                    );
                                                    paint_cover_or_placeholder(
                                                        ui, None, rect, *index,
                                                    );
                                                }
                                                ui.vertical(|ui| {
                                                    open = ui
                                                        .selectable_label(
                                                            selected,
                                                            RichText::new(album)
                                                                .size(18.0)
                                                                .strong(),
                                                        )
                                                        .clicked();
                                                    ui.label(
                                                        RichText::new(format!(
                                                            "Album  ·  {}",
                                                            display_metadata(&artist)
                                                        ))
                                                        .size(14.0)
                                                        .color(Color32::from_rgb(182, 185, 196)),
                                                    );
                                                });
                                            });
                                            if open {
                                                album_open = Some(album.clone());
                                            }
                                            false
                                        }
                                    };
                                    let _ = clicked;
                                }
                            }
                        });
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            for (key, icon_key) in [("Down", "arrow_down"), ("Up", "arrow_up")] {
                                ui.label(RichText::new(key).small().color(Color32::GRAY));
                                if let Some(icon) = self.ui_icons.get(icon_key) {
                                    ui.add(
                                        egui::Image::new(icon).fit_to_exact_size(Vec2::splat(16.0)),
                                    );
                                }
                            }
                            ui.label(
                                RichText::new("Enter: search · Esc: close")
                                    .small()
                                    .color(Color32::GRAY),
                            );
                        });
                    });
            });
        if let Some((index, queue)) = play {
            self.query = query.clone();
            self.remember_search(&query);
            self.page = Page::Home;
            self.search_suggestions_open = false;
            self.search_suggestions_focused = false;
            let start = queue.iter().position(|&item| item == index).unwrap_or(0);
            self.play_indices(queue, start);
        }
        if let Some(artist) = artist_open {
            self.remember_search(&query);
            self.selected_artist = Some(artist);
            self.page = Page::ArtistDetail;
            self.search_suggestions_open = false;
            self.search_suggestions_focused = false;
        }
        if let Some(album) = album_open {
            self.remember_search(&query);
            self.selected_album = Some(album);
            self.page = Page::AlbumDetail;
            self.search_suggestions_open = false;
            self.search_suggestions_focused = false;
        }
        if ask_ai {
            self.remember_search(&query);
            self.search_ai_results.clear();
            self.search_ai_answer.clear();
            self.search_ai_pending = true;
            self.search_ai_open = true;
            self.ai_mix_prompt = query;
            self.search_suggestions_open = false;
            self.search_suggestions_focused = false;
            self.request_ai_mix(ctx);
            if !self.ai_mix_pending {
                self.search_ai_pending = false;
                if self.search_ai_answer.is_empty() {
                    self.search_ai_answer = self.ai_mix_status.clone();
                }
            }
        }
        if let Some(recent) = history_open {
            self.query = recent.clone();
            self.remember_search(&recent);
            self.search_committed = true;
            self.page = Page::Home;
            self.search_suggestions_open = false;
            self.search_suggestions_focused = false;
        }
        if let Some(recent) = history_delete {
            self.search_history.retain(|item| item != &recent);
            self.save_state();
        }
    }

    fn show_search_ai_window(&mut self, context: &egui::Context) {
        if !self.search_ai_open {
            return;
        }
        let mut open = true;
        let mut play = None;
        egui::Window::new("Ask AI · Library search")
            .id(egui::Id::new("search-ai-answer-window"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(560.0)
            .default_height(420.0)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(context, |ui| {
                ui.horizontal(|ui| {
                    if let Some(icon) = self.ui_icons.get("sparkles") {
                        ui.add(egui::Image::new(icon).fit_to_exact_size(Vec2::splat(24.0)));
                    }
                    ui.vertical(|ui| {
                        ui.heading("Your music question");
                        ui.label(
                            RichText::new(self.query.trim())
                                .small()
                                .color(Color32::GRAY),
                        );
                    });
                });
                ui.separator();
                if self.search_ai_pending {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Looking through your library and writing an answer…");
                    });
                } else {
                    egui::ScrollArea::vertical()
                        .max_height(200.0)
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(if self.search_ai_answer.is_empty() {
                                    "I couldn't generate an answer. Please try again."
                                } else {
                                    &self.search_ai_answer
                                })
                                .size(16.0),
                            );
                        });
                    ui.add_space(12.0);
                    ui.heading("Songs from your library");
                    if self.search_ai_results.is_empty() {
                        ui.label(
                            RichText::new("No matching songs were found in your library.")
                                .small()
                                .color(Color32::GRAY),
                        );
                    } else {
                        for index in self.search_ai_results.clone() {
                            let Some((title, artist, album)) =
                                self.tracks.get(index).map(|track| {
                                    (
                                        track.title.clone(),
                                        track.artist.clone(),
                                        track.album.clone(),
                                    )
                                })
                            else {
                                continue;
                            };
                            ui.horizontal(|ui| {
                                if let Some(cover) = self.artwork_texture(ui.ctx(), index) {
                                    ui.add(
                                        egui::Image::new(&cover)
                                            .fit_to_exact_size(Vec2::splat(44.0)),
                                    );
                                } else {
                                    let (rect, _) = ui.allocate_exact_size(
                                        Vec2::splat(44.0),
                                        egui::Sense::hover(),
                                    );
                                    paint_cover_or_placeholder(ui, None, rect, index);
                                }
                                ui.vertical(|ui| {
                                    ui.label(RichText::new(title).strong());
                                    ui.label(
                                        RichText::new(format!(
                                            "{} · {}",
                                            display_metadata(&artist),
                                            display_metadata(&album)
                                        ))
                                        .small()
                                        .color(Color32::GRAY),
                                    );
                                });
                                if ui.button("Play").clicked() {
                                    play = Some(index);
                                }
                            });
                        }
                    }
                }
            });
        self.search_ai_open = open;
        if let Some(index) = play {
            if let Some(position) = self
                .search_ai_results
                .iter()
                .position(|&item| item == index)
            {
                self.play_indices(self.search_ai_results.clone(), position);
                self.search_ai_open = false;
            }
        }
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
                            let cover =
                                self.artist_artwork_texture(ui.ctx(), artist).or_else(|| {
                                    tracks
                                        .first()
                                        .and_then(|&index| self.artwork_texture(ui.ctx(), index))
                                });
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
                                {
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
                                }
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
                                let painter = ui.painter();
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
                                let title_response = ui
                                    .interact(
                                        title_rect,
                                        ui.make_persistent_id(("artist-title", position)),
                                        egui::Sense::click(),
                                    )
                                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                                text_painter.text(
                                    egui::pos2(text_left, title_rect.center().y),
                                    egui::Align2::LEFT_CENTER,
                                    &artist,
                                    egui::FontId::proportional(18.0),
                                    Color32::WHITE,
                                );
                                if title_response.clicked() {
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
                                        egui::Button::image_and_text(
                                            egui::Image::new(&self.ui_icons["add"])
                                                .fit_to_exact_size(Vec2::splat(16.0)),
                                            "Save as playlist",
                                        ),
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
                RichText::new("A personal mix shaped by what you play, finish, replay, like, and skip")
                    .color(Color32::from_rgb(177, 174, 181)),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_sized([116.0, 40.0], egui::Button::new("⇄  Shuffle mix"))
                    .clicked()
                {
                    self.play_shuffled_indices(indices.to_vec());
                }
                if ui
                    .add_sized(
                        [104.0, 40.0],
                        egui::Button::new("▶  Play mix").fill(accent_color(&self.accent)),
                    )
                    .clicked()
                {
                    self.play_indices(indices.to_vec(), 0);
                }
            });
        });
        ui.add_space(12.0);
        let previous_view = self.view_mode;
        self.view_mode = ViewMode::List;
        self.show_track_collection(ui, indices, "quick-picks-mix");
        self.view_mode = previous_view;
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
                .add_enabled_ui(!tracks.is_empty(), |ui| {
                    icon_text_button(ui, self.ui_icons.get("shuffle"), "Shuffle", false).clicked()
                })
                .inner
            {
                shuffle = true;
            }
            if ui
                .add_enabled_ui(!tracks.is_empty(), |ui| {
                    icon_text_button(ui, self.ui_icons.get("add_to_queue"), "Add to queue", false)
                        .clicked()
                })
                .inner
            {
                enqueue = true;
            }
            if icon_text_button(ui, self.ui_icons.get("add"), "Add now playing", false).clicked() {
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
            if icon_text_button(ui, self.ui_icons.get("add"), "Create playlist", false).clicked() {
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
        let mut export: Option<(usize, PlaylistExportFormat)> = None;
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
                    if icon_text_button(ui, self.ui_icons.get("shuffle"), "Shuffle", false)
                        .clicked()
                    {
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
                if icon_text_button(ui, self.ui_icons.get("shuffle"), "Shuffle and play", false)
                    .clicked()
                {
                    open = Some(index);
                    shuffle_play = true;
                    ui.close_menu();
                }
                if ui.button("♫  Add playlist to queue").clicked() {
                    open = Some(index);
                    enqueue = true;
                    ui.close_menu();
                }
                if icon_text_button(ui, self.ui_icons.get("next"), "Play playlist next", false)
                    .clicked()
                {
                    open = Some(index);
                    play_next = true;
                    ui.close_menu();
                }
                if icon_text_button(ui, self.ui_icons.get("copy"), "Copy track list", false)
                    .clicked()
                {
                    copy_track_list = Some(index);
                    ui.close_menu();
                }
                ui.menu_button("⇩  Export playlist…", |ui| {
                    if ui.button("Folder with audio files…").clicked() {
                        export = Some((index, PlaylistExportFormat::Folder));
                        ui.close_menu();
                    }
                    if ui.button("ZIP archive…").clicked() {
                        export = Some((index, PlaylistExportFormat::Zip));
                        ui.close_menu();
                    }
                    if ui.button("M3U8 playlist…").clicked() {
                        export = Some((index, PlaylistExportFormat::M3u8));
                        ui.close_menu();
                    }
                });
                if ui.button("Remove missing tracks").clicked() {
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
                if icon_text_button(
                    ui,
                    self.ui_icons.get("add"),
                    "Add currently playing song",
                    false,
                )
                .clicked()
                {
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
        if let Some((index, format)) = export {
            if let Some(playlist) = self.playlists.get(index) {
                let name = playlist.name.clone();
                let tracks = playlist.tracks.clone();
                let result = match format {
                    PlaylistExportFormat::Folder => rfd::FileDialog::new()
                        .set_title("Choose where to export the playlist folder")
                        .pick_folder()
                        .map(|destination| export_playlist_folder(&name, &tracks, &destination)),
                    PlaylistExportFormat::Zip => {
                        let file_name = format!("{}.zip", safe_export_name(&name));
                        rfd::FileDialog::new()
                            .add_filter("ZIP archive", &["zip"])
                            .set_file_name(&file_name)
                            .save_file()
                            .map(|destination| export_playlist_zip(&name, &tracks, &destination))
                    }
                    PlaylistExportFormat::M3u8 => {
                        let file_name = format!("{}.m3u8", safe_export_name(&name));
                        rfd::FileDialog::new()
                            .add_filter("M3U8 playlist", &["m3u8"])
                            .set_file_name(&file_name)
                            .save_file()
                            .map(|destination| write_m3u8_file(&tracks, &destination))
                    }
                };
                if let Some(result) = result {
                    self.status = match result {
                        Ok(message) => message,
                        Err(error) => format!("Could not export playlist: {error}"),
                    };
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
                "Changes save automatically. Move a slider, then choose Save slider changes to apply it.",
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
        ui.horizontal_wrapped(|ui| {
            for category in [
                "Appearance",
                "Playback",
                "Library",
                "Discover",
                "AI",
                "Advanced",
            ] {
                if ui
                    .selectable_label(self.settings_category == category, category)
                    .clicked()
                {
                    self.settings_category = category.to_owned();
                    self.settings_advanced = matches!(category, "Discover" | "AI" | "Advanced");
                }
            }
        });
        ui.separator();
        let appearance_settings = self.settings_category == "Appearance";
        let playback_settings = self.settings_category == "Playback";
        let library_settings = self.settings_category == "Library";
        let discover_settings = self.settings_category == "Discover";
        let ai_settings = self.settings_category == "AI";
        let advanced_settings = self.settings_category == "Advanced";
        egui::Frame::new()
            .fill(Color32::from_rgb(17, 19, 26))
            .corner_radius(egui::CornerRadius::same(12))
            .inner_margin(egui::Margin::symmetric(16, 14))
            .show(ui, |ui| {
        egui::Grid::new("settings-grid")
            .num_columns(2)
            .spacing([24.0, 18.0])
            .show(ui, |ui| {
                if appearance_settings {
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
                ui.label(RichText::new("Window startup").strong());
                ui.checkbox(&mut draft.start_maximized, "Open Lilt maximized");
                ui.end_row();
                ui.label(RichText::new("Motion").strong());
                ui.checkbox(&mut draft.animations_enabled, "Enable smooth transitions and hover effects");
                ui.end_row();
                ui.label(RichText::new("Online artwork").strong());
                ui.checkbox(&mut draft.fetch_online_artwork, "Fetch missing album and artist covers in the background");
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
                }
                if advanced_settings {
                ui.label(RichText::new("Developer console").strong());
                ui.checkbox(&mut draft.developer_console_enabled, "Enable Developer Output panel");
                ui.end_row();
                }
                if discover_settings {
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
                }
                if playback_settings {
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
                }
                if library_settings {
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
                }
                if library_settings {
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
                        if icon_text_button(ui, self.ui_icons.get("add"), "Add folder", false).clicked() {
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
                }
                if library_settings {
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
                ui.label(RichText::new("Lyrics").strong());
                ui.checkbox(&mut draft.lyrics_enabled, "Enable optional .lrc lyrics support");
                ui.end_row();
                }
            });

            if ai_settings {
        ui.add_space(14.0);
        ui.separator();
        ui.heading("Mix Studio");
        ui.label(RichText::new("Use your own Google AI key for Mix Studio, search answers, or optional library review. The key is stored in Windows Credential Manager. AI requests send a prompt and compact music labels from your library; file paths and audio are never sent. Google usage limits and billing may apply.").small().color(Color32::GRAY));
        ui.horizontal(|ui| {
            ui.label("Google AI key");
            ui.add(egui::TextEdit::singleline(&mut self.google_ai_key_draft).password(true).hint_text("Paste your API key"));
            if ui.button(if self.google_ai_key_saved { "Update key" } else { "Save key" }).clicked() {
                match write_google_ai_key(&self.google_ai_key_draft) {
                    Ok(()) => { self.google_ai_key_saved = true; self.google_ai_key_draft.clear(); self.ai_mix_status = "Google AI key saved securely on this device.".into(); }
                    Err(error) => self.ai_mix_status = error,
                }
            }
            if ui.add_enabled(self.google_ai_key_saved, egui::Button::new("Remove")).clicked() {
                match delete_google_ai_key() {
                    Ok(()) => { self.google_ai_key_saved = false; self.google_ai_key_draft.clear(); self.ai_mix_status = "Google AI key removed.".into(); }
                    Err(error) => self.ai_mix_status = error,
                }
            }
        });
        ui.hyperlink_to("Get a Google AI key", "https://aistudio.google.com/app/apikey");
        if !self.ai_mix_status.is_empty() { ui.label(RichText::new(&self.ai_mix_status).small().color(accent_color(&self.accent))); }
        ui.add_space(18.0);
        ui.separator();
        ui.heading("AI library review");
        ui.label(RichText::new("Ask Google to flag likely duplicate recordings and high-confidence artist-credit mistakes. It checks compact title, artist, and album labels across your library; file paths and audio are not sent. Nothing is changed automatically, and you should review suggestions before editing tags.").small().color(Color32::GRAY));
        ui.checkbox(&mut draft.ai_audit_check_duplicates, "Look for duplicate songs and alternate copies");
        ui.checkbox(&mut draft.ai_audit_check_artist_mismatches, "Look for songs credited to the wrong artist");
        let can_audit = self.google_ai_key_saved && !self.ai_audit_pending && !self.tracks.is_empty();
        let mut run_ai_audit = false;
        ui.horizontal(|ui| {
            if let Some(icon) = self.ui_icons.get("sparkles") {
                ui.add(egui::Image::new(icon).fit_to_exact_size(Vec2::splat(18.0)));
            }
            run_ai_audit = ui.add_enabled(can_audit, egui::Button::new("Review my library")).clicked();
        });
        if run_ai_audit {
            self.start_ai_library_audit(ui.ctx());
        }
        if self.ai_audit_pending {
            ui.horizontal(|ui| { ui.spinner(); ui.label(&self.ai_audit_status); });
        } else if !self.ai_audit_status.is_empty() {
            ui.label(RichText::new(&self.ai_audit_status).color(accent_color(&self.accent)));
        }
        for finding in &self.ai_audit_findings {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                let heading = if finding.kind == "artist_mismatch" { "Possible artist-credit mistake" } else { "Possible duplicate recording" };
                ui.label(RichText::new(heading).strong());
                for id in &finding.track_ids {
                    if let Some(track) = self.tracks.get(*id) {
                        ui.label(format!("{} · {}", track.title, display_metadata(&track.artist)));
                    }
                }
                if !finding.suggested_artist.is_empty() {
                    ui.label(format!("Likely artist: {}", finding.suggested_artist));
                }
                ui.label(RichText::new(&finding.explanation).small().color(Color32::GRAY));
            });
        }
        }
        if advanced_settings {
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

            }
        });

        ui.add_space(8.0);
        ui.label(RichText::new("Online album and artist covers").strong());
        ui.label(RichText::new("When enabled, Lilt looks up missing album artwork and artist portraits in the background and keeps a local cache. Album searches use MusicBrainz and Cover Art Archive; artist photos use Wikimedia. Only artist and album names are sent to those services.").small().color(Color32::GRAY));
        ui.label(
            RichText::new(if !self.online_artwork_status.is_empty() {
                self.online_artwork_status.as_str()
            } else if self.fetch_online_artwork {
                "Background cover lookup is enabled."
            } else {
                "Background cover lookup is off."
            })
            .small()
            .color(accent_color(&self.accent)),
        );
        let staged_sliders = (
            draft.ui_scale,
            draft.discovery,
            draft.recommendation_variety,
            draft.recommendation_size,
            draft.volume,
        );
        let sliders_pending = (draft.ui_scale - self.ui_scale).abs() > f32::EPSILON
            || (draft.discovery - self.discovery).abs() > f32::EPSILON
            || (draft.recommendation_variety - self.recommendation_variety).abs() > f32::EPSILON
            || draft.recommendation_size != self.recommendation_size
            || (draft.volume - self.volume).abs() > f32::EPSILON;
        ui.horizontal(|ui| {
            save = ui
                .add_enabled(sliders_pending, egui::Button::new("Save slider changes"))
                .clicked();
            discard = ui
                .add_enabled(sliders_pending, egui::Button::new("Discard sliders"))
                .clicked();
        });
        let manual_save = save;
        let auto_save = draft.view_mode != self.view_mode
            || draft.album_view_mode != self.album_view_mode
            || draft.artist_view_mode != self.artist_view_mode
            || draft.accent != self.accent
            || draft.shuffle != self.shuffle
            || draft.repeat != self.repeat
            || draft.show_queue_on_start != !self.queue_collapsed
            || draft.compact_mode != self.compact_mode
            || draft.start_maximized != self.start_maximized
            || draft.animations_enabled != self.animations_enabled
            || draft.developer_console_enabled != self.developer_console_enabled
            || draft.ai_audit_check_duplicates != self.ai_audit_check_duplicates
            || draft.ai_audit_check_artist_mismatches != self.ai_audit_check_artist_mismatches
            || draft.fetch_online_artwork != self.fetch_online_artwork
            || draft.lyrics_enabled != self.lyrics_enabled
            || draft.prefer_unplayed != self.prefer_unplayed
            || draft.scan_rate_per_second != self.scan_rate_per_second;
        if auto_save && !manual_save {
            draft.ui_scale = self.ui_scale;
            draft.discovery = self.discovery;
            draft.recommendation_variety = self.recommendation_variety;
            draft.recommendation_size = self.recommendation_size;
            draft.volume = self.volume;
            save = true;
        }
        self.settings_draft = Some(draft.clone());
        if save {
            self.view_mode = draft.view_mode;
            self.album_view_mode = draft.album_view_mode;
            self.artist_view_mode = draft.artist_view_mode;
            self.accent = draft.accent.clone();
            self.shuffle = draft.shuffle;
            self.repeat = draft.repeat;
            self.queue_collapsed = !draft.show_queue_on_start;
            self.compact_mode = draft.compact_mode;
            self.start_maximized = draft.start_maximized;
            self.animations_enabled = draft.animations_enabled;
            self.developer_console_enabled = draft.developer_console_enabled;
            self.ai_audit_check_duplicates = draft.ai_audit_check_duplicates;
            self.ai_audit_check_artist_mismatches = draft.ai_audit_check_artist_mismatches;
            if !self.developer_console_enabled {
                self.developer_output_open = false;
            }
            let lyrics_setting_changed = self.lyrics_enabled != draft.lyrics_enabled;
            let artwork_fetch_changed = self.fetch_online_artwork != draft.fetch_online_artwork;
            self.fetch_online_artwork = draft.fetch_online_artwork;
            if artwork_fetch_changed {
                self.online_artwork_prompt_seen = true;
                if self.fetch_online_artwork {
                    self.start_online_artwork_fetch();
                }
            }
            self.lyrics_enabled = draft.lyrics_enabled;
            if lyrics_setting_changed {
                if self.lyrics_enabled {
                    if let Some(path) = self
                        .current
                        .and_then(|position| self.queue.get(position))
                        .and_then(|&index| self.tracks.get(index))
                        .map(|track| track.path.clone())
                    {
                        self.load_lyrics_for(&path);
                    }
                } else {
                    self.lyrics = None;
                    self.lyrics_error = None;
                    if self.page == Page::Lyrics {
                        self.page = Page::Home;
                    }
                }
            }
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
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Maximized(self.start_maximized));
            self.recommendations_dirty = true;
            self.home_cache_dirty = true;
            self.settings_draft = None;
            self.save_state();
            self.status = "Settings applied and saved.".into();
        } else if discard {
            self.settings_draft = None;
            self.status = "Unapplied slider changes discarded.".into();
        }
        if auto_save && !manual_save {
            draft.ui_scale = staged_sliders.0;
            draft.discovery = staged_sliders.1;
            draft.recommendation_variety = staged_sliders.2;
            draft.recommendation_size = staged_sliders.3;
            draft.volume = staged_sliders.4;
            self.settings_draft = Some(draft);
        }
    }

    fn recommendation_indices(&mut self) -> Vec<usize> {
        if self.recommendations_dirty {
            self.recommendations = self.compute_recommendations();
            self.recommendations_dirty = false;
        }
        let mut result = Vec::with_capacity(self.recommendations.len());
        let mut seen = HashSet::new();
        if self.ai_recommendations_enabled {
            for &index in &self.ai_recommendation_indices {
                let available = self.tracks.get(index).is_some_and(|track| {
                    !self.disliked_songs.contains(&track.path)
                        && !self.excluded_tracks.contains(&track.path)
                });
                if available && seen.insert(index) {
                    result.push(index);
                }
            }
        }
        result.extend(
            self.recommendations
                .iter()
                .copied()
                .filter(|index| seen.insert(*index)),
        );
        result
    }

    fn compute_recommendations(&self) -> Vec<usize> {
        self.mix_context().compute_recommendations()
    }

    fn request_ai_mix(&mut self, context: &egui::Context) {
        if self.ai_mix_pending {
            return;
        }
        let api_key = match read_google_ai_key() {
            Ok(Some(key)) => key,
            Ok(None) => {
                self.ai_mix_status = "Add your Google AI key in Settings → AI.".into();
                self.google_ai_key_saved = false;
                return;
            }
            Err(error) => {
                self.ai_mix_status = error;
                return;
            }
        };
        let theme = if self.ai_mix_prompt.trim().is_empty() {
            "a balanced mix based on my music taste".to_string()
        } else {
            self.ai_mix_prompt.chars().take(120).collect::<String>()
        };
        if !self.ai_mix_include_liked
            && !self.ai_mix_include_played
            && !self.ai_mix_include_unplayed
        {
            self.ai_mix_status = "Choose at least one music group in Mix options.".into();
            return;
        }
        // Include the whole available local catalog. The old 48-track cap meant
        // large libraries only exposed their first recommendation cluster.
        let candidates = (0..self.tracks.len())
            .filter_map(|index| {
                let track = self.tracks.get(index)?;
                if self.excluded_tracks.contains(&track.path)
                    || self.disliked_songs.contains(&track.path)
                {
                    return None;
                }
                let liked = self.liked_songs.contains(&track.path);
                let played = self.history.contains(&track.path);
                let followed_artist = artist_names(&track.artist)
                    .iter()
                    .any(|artist| self.liked_artists.contains(&artist.to_lowercase()));
                let saved_album = self.liked_albums.contains(&track.album.to_lowercase());
                let stats = self.track_engagement.get(&track.path).copied().unwrap_or_default();
                let heard_percent = if track.duration > 0.0 {
                    (stats.listened_seconds / (track.duration * stats.plays.max(1) as f64) * 100.0)
                        .round()
                        .clamp(0.0, 100.0) as u8
                } else {
                    0
                };
                let included = (liked && self.ai_mix_include_liked)
                    || (played && self.ai_mix_include_played)
                    || (!liked && !played && self.ai_mix_include_unplayed);
                if !included {
                    return None;
                }
                Some(AiMixCandidate {
                    index,
                    title: compact_metadata(&track.title, 64),
                    artist: compact_metadata(&display_credit(&track.artist), 48),
                    genre: compact_metadata(&track.genre, 32),
                    familiarity: if liked {
                        "L"
                    } else if played {
                        "P"
                    } else {
                        "N"
                    },
                    engagement: format!(
                        "l{}f{}a{}h{}p{}c{}s{}r{}b{}",
                        u8::from(liked),
                        u8::from(followed_artist),
                        u8::from(saved_album),
                        heard_percent,
                        stats.plays.min(99),
                        stats.completions.min(9),
                        stats.skips.min(9),
                        stats.replays.min(9),
                        stats.seek_backs.min(9),
                    ),
                })
            })
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            self.ai_mix_status = "Add a few songs to your library first.".into();
            return;
        }

        self.ai_mix_pending = true;
        self.ai_mix_status = format!(
            "Making a mix from all {} available tracks…",
            candidates.len()
        );
        let sender = self.ai_mix_sender.clone();
        let repaint = context.clone();
        let model = self.google_ai_model;
        let thinking = self.google_ai_thinking;
        let include_answer = self.search_ai_pending;
        let mix_size = if self.search_ai_pending {
            5
        } else {
            self.ai_mix_size
        };
        thread::spawn(move || {
            let result = generate_google_ai_mix(
                &api_key,
                &theme,
                &candidates,
                model,
                thinking,
                mix_size,
                include_answer,
            );
            let _ =
                sender.send(result.map(|(indices, usage, answer)| (theme, indices, usage, answer)));
            request_repaint_when_focused(&repaint);
        });
    }

    fn poll_ai_mix(&mut self) {
        while let Ok(result) = self.ai_mix_receiver.try_recv() {
            self.ai_mix_pending = false;
            match result {
                Ok((theme, indices, usage, answer)) => {
                    self.google_ai_last_usage = Some(usage);
                    self.google_ai_usage.prompt_tokens = self
                        .google_ai_usage
                        .prompt_tokens
                        .saturating_add(usage.prompt_tokens);
                    self.google_ai_usage.output_tokens = self
                        .google_ai_usage
                        .output_tokens
                        .saturating_add(usage.output_tokens);
                    self.google_ai_usage.thinking_tokens = self
                        .google_ai_usage
                        .thinking_tokens
                        .saturating_add(usage.thinking_tokens);
                    self.google_ai_usage.total_tokens = self
                        .google_ai_usage
                        .total_tokens
                        .saturating_add(usage.total_tokens);
                    self.google_ai_requests = self.google_ai_requests.saturating_add(1);
                    self.ai_mix_prompt = theme.clone();
                    self.ai_mix_results = indices.clone();
                    if self.search_ai_pending {
                        self.search_ai_results = indices.iter().copied().take(5).collect();
                        self.search_ai_answer = answer.unwrap_or_else(|| {
                            "I found a few songs in your library that fit your search.".into()
                        });
                        self.search_ai_pending = false;
                        self.ai_mix_status = format!(
                            "AI found {} tracks for your search.",
                            self.search_ai_results.len()
                        );
                        self.search_suggestions_open = false;
                    } else if self.ai_recommendation_mode {
                        self.ai_recommendation_indices = indices;
                        self.recommendations_dirty = false;
                        self.ai_mix_status = format!(
                            "Updated For you with {} AI-assisted picks.",
                            self.ai_recommendation_indices.len()
                        );
                    } else {
                        self.ai_mix_status =
                            format!("{} tracks ready for {theme}.", self.ai_mix_results.len());
                    }
                    self.ai_recommendation_mode = false;
                    self.save_state();
                }
                Err(error) => {
                    self.ai_mix_status = error.clone();
                    self.status = error.clone();
                    self.ai_recommendation_mode = false;
                    self.search_ai_pending = false;
                    if self.search_ai_open {
                        self.search_ai_answer = error.clone();
                    }
                }
            }
        }
    }

    fn start_ai_library_audit(&mut self, context: &egui::Context) {
        if self.ai_audit_pending {
            return;
        }
        let key = match read_google_ai_key() {
            Ok(Some(key)) => key,
            Ok(None) => {
                self.ai_audit_status = "Add a Google AI key in Settings → AI first.".into();
                self.google_ai_key_saved = false;
                return;
            }
            Err(error) => {
                self.ai_audit_status = error;
                return;
            }
        };
        if !self.ai_audit_check_duplicates && !self.ai_audit_check_artist_mismatches {
            self.ai_audit_status = "Choose at least one review option.".into();
            return;
        }
        let catalog = self
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, track)| !self.excluded_tracks.contains(&track.path))
            .map(|(id, track)| {
                format!(
                    "{id}\t{}\t{}\t{}",
                    compact_tsv_field(&compact_metadata(&track.title, 70)),
                    compact_tsv_field(&compact_metadata(&display_credit(&track.artist), 55)),
                    compact_tsv_field(&compact_metadata(&track.album, 55)),
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        if catalog.is_empty() {
            self.ai_audit_status = "Add music to your library before reviewing it.".into();
            return;
        }
        self.ai_audit_pending = true;
        self.ai_audit_status = format!("Reviewing {} catalog entries…", self.tracks.len());
        self.ai_audit_findings.clear();
        let check_duplicates = self.ai_audit_check_duplicates;
        let check_artist = self.ai_audit_check_artist_mismatches;
        let model = self.google_ai_model;
        let thinking = self.google_ai_thinking;
        let sender = self.ai_audit_sender.clone();
        let repaint = context.clone();
        thread::spawn(move || {
            let result = generate_google_ai_audit(
                &key,
                &catalog,
                check_duplicates,
                check_artist,
                model,
                thinking,
            );
            let _ = sender.send(result);
            request_repaint_when_focused(&repaint);
        });
    }

    fn poll_ai_library_audit(&mut self) {
        while let Ok(result) = self.ai_audit_receiver.try_recv() {
            self.ai_audit_pending = false;
            match result {
                Ok((findings, usage)) => {
                    self.google_ai_last_usage = Some(usage);
                    self.google_ai_usage.prompt_tokens = self
                        .google_ai_usage
                        .prompt_tokens
                        .saturating_add(usage.prompt_tokens);
                    self.google_ai_usage.output_tokens = self
                        .google_ai_usage
                        .output_tokens
                        .saturating_add(usage.output_tokens);
                    self.google_ai_usage.thinking_tokens = self
                        .google_ai_usage
                        .thinking_tokens
                        .saturating_add(usage.thinking_tokens);
                    self.google_ai_usage.total_tokens = self
                        .google_ai_usage
                        .total_tokens
                        .saturating_add(usage.total_tokens);
                    self.google_ai_requests = self.google_ai_requests.saturating_add(1);
                    self.ai_audit_findings = findings;
                    self.ai_audit_status = if self.ai_audit_findings.is_empty() {
                        "Review complete. No high-confidence issues were found.".into()
                    } else {
                        format!(
                            "Review complete · {} item(s) need a look.",
                            self.ai_audit_findings.len()
                        )
                    };
                    self.save_state();
                }
                Err(error) => {
                    self.ai_audit_status = error.clone();
                    self.status = error;
                }
            }
        }
    }

    fn capture_status_output(&mut self, context: &egui::Context) {
        if self.status.is_empty() || self.status == self.last_status_seen {
            return;
        }
        self.last_status_seen.clone_from(&self.status);
        let message = compact_metadata(&self.status, 500);
        let lower = message.to_lowercase();
        let is_error = [
            "error",
            "failed",
            "could not",
            "unable",
            "invalid",
            "blocked",
            "stopped before",
            "no candidate",
            "no text part",
        ]
        .iter()
        .any(|needle| lower.contains(needle));
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let level = if is_error { "ERROR" } else { "INFO" };
        self.developer_output
            .push_back(format!("{time} [{level}] {message}"));
        while self.developer_output.len() > 250 {
            self.developer_output.pop_front();
        }
        if is_error {
            self.error_notification = Some((message, Instant::now()));
            context.request_repaint_after(Duration::from_secs(6));
        }
    }

    fn request_ai_recommendations(&mut self, context: &egui::Context) {
        if !self.ai_recommendations_enabled {
            return;
        }
        self.ai_mix_prompt = "Recommend a balanced For you mix using familiar artists, liked tracks, and a few new discoveries.".into();
        self.ai_recommendation_mode = true;
        self.request_ai_mix(context);
        if !self.ai_mix_pending {
            self.ai_recommendation_mode = false;
        }
    }

    fn store_ai_mix_as_playlist(&mut self) {
        if self.ai_mix_results.is_empty() {
            return;
        }
        let name = if self.ai_mix_prompt.trim().is_empty() {
            "Mix Studio playlist".to_string()
        } else {
            format!("Mix · {}", self.ai_mix_prompt.trim())
        };
        let tracks = self
            .ai_mix_results
            .iter()
            .filter_map(|&index| self.tracks.get(index).map(|track| track.path.clone()))
            .collect::<Vec<_>>();
        self.playlists.push(Playlist { name, tracks });
        self.save_state();
        self.status = "Saved the Mix Studio picks as a playlist.".into();
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
                "search",
                include_bytes!("../assets/icons/ic-search-24px.svg"),
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
            (
                "smart_mixes",
                include_bytes!("../assets/icons/ic-smart-mixes-24px.svg"),
            ),
            (
                "sparkles",
                include_bytes!("../assets/icons/ic-auto-awesome-24px.svg"),
            ),
            (
                "arrow_down",
                include_bytes!("../assets/icons/ic-keyboard-arrow-down-24px.svg"),
            ),
            (
                "arrow_up",
                include_bytes!("../assets/icons/ic-keyboard-arrow-up-24px.svg"),
            ),
            ("add", include_bytes!("../assets/icons/ic-add-24px.svg")),
            (
                "queue",
                include_bytes!("../assets/icons/ic-queue-music-24px.svg"),
            ),
            (
                "copy",
                include_bytes!("../assets/icons/ic-content-copy-24px.svg"),
            ),
            (
                "open_in_new",
                include_bytes!("../assets/icons/ic-open-in-new-24px.svg"),
            ),
            (
                "folder_open",
                include_bytes!("../assets/icons/ic-folder-open-24px.svg"),
            ),
            (
                "music_note",
                include_bytes!("../assets/icons/ic-music-note-24px.svg"),
            ),
            (
                "add_to_queue",
                include_bytes!("../assets/icons/ic-add-to-queue-24px.svg"),
            ),
            (
                "more",
                include_bytes!("../assets/icons/ic-more-vert-24px.svg"),
            ),
            (
                "delete",
                include_bytes!("../assets/icons/ic-delete-24px.svg"),
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

    fn artist_artwork_texture(
        &mut self,
        context: &egui::Context,
        artist: &str,
    ) -> Option<TextureHandle> {
        let texture_key = artist_art_texture_key(artist);
        if let Some(texture) = self.textures.get(&texture_key).cloned() {
            return Some(texture);
        }
        if !context.input(|input| input.focused)
            || self.artwork_pending.len() >= 4
            || self.artwork_pending.contains(&texture_key)
            || self.artwork_failed.contains(&texture_key)
        {
            return None;
        }
        let bytes = self
            .artist_artworks
            .get(&artist.trim().to_lowercase())?
            .clone();
        let sender = self.artwork_sender.clone();
        let request_path = texture_key.clone();
        self.artwork_pending.insert(texture_key);
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
        context.request_repaint_after(Duration::from_millis(50));
        None
    }
}


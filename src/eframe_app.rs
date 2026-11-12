impl eframe::App for PlayerApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let window_focused = ctx.input(|input| input.focused);
        self.poll_update_messages();
        self.load_ui_icons(ctx);
        #[cfg(target_os = "windows")]
        self.initialize_media_controls(frame, ctx);
        if window_focused {
            self.poll_scan();
            self.poll_home_job();
            self.poll_duplicate_scan();
            self.poll_artwork(ctx);
            self.poll_online_artwork(ctx);
            self.poll_ai_mix();
            self.poll_ai_library_audit();
        }
        // Windows media-key events arrive through the system media session and
        // must be serviced even while Lilt is in the background.
        self.poll_media_key_commands();
        self.capture_status_output(ctx);
        if !self.artwork_pending.is_empty() {
            if window_focused {
                ctx.request_repaint_after(Duration::from_millis(50));
            }
        }
        // Playback must keep advancing while the window is unfocused, even though
        // library scans, artwork, and other UI work stay paused until focus returns.
        self.update_playback_transition();
        self.record_playback_sample();
        if self.sink.as_ref().is_some_and(Sink::empty) {
            self.next_track_automatically();
        }
        #[cfg(target_os = "windows")]
        self.update_media_session();
        let can_use_shortcuts = !ctx.wants_keyboard_input();
        let shortcuts = ctx.input(|input| {
            let ctrl = input.modifiers.ctrl;
            (
                can_use_shortcuts
                    && (input.key_pressed(egui::Key::Space)
                        || (ctrl && input.key_pressed(egui::Key::P))),
                can_use_shortcuts && input.key_pressed(egui::Key::ArrowRight),
                can_use_shortcuts && input.key_pressed(egui::Key::ArrowLeft),
                (ctrl && input.key_pressed(egui::Key::F))
                    || (can_use_shortcuts
                        && !input.modifiers.any()
                        && input.key_pressed(egui::Key::Slash)),
                ctrl && input.key_pressed(egui::Key::Num1),
                ctrl && input.key_pressed(egui::Key::Num2),
                ctrl && input.key_pressed(egui::Key::Num3),
                ctrl && input.key_pressed(egui::Key::Num4),
                ctrl && input.key_pressed(egui::Key::Num5),
                ctrl && input.key_pressed(egui::Key::Num6),
                ctrl && input.key_pressed(egui::Key::Num7),
                ctrl && input.key_pressed(egui::Key::Num8),
                ctrl && input.key_pressed(egui::Key::Num9),
                ctrl && input.key_pressed(egui::Key::R),
                ctrl && input.key_pressed(egui::Key::O),
                ctrl && input.key_pressed(egui::Key::L),
                ctrl && input.key_pressed(egui::Key::Q),
                !input.modifiers.any() && input.key_pressed(egui::Key::Escape),
                input.modifiers.alt && input.key_pressed(egui::Key::ArrowUp),
                input.modifiers.alt && input.key_pressed(egui::Key::ArrowDown),
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
            let slash_shortcut =
                ctx.input(|input| !input.modifiers.any() && input.key_pressed(egui::Key::Slash));
            if slash_shortcut {
                ctx.input_mut(|input| {
                    input.consume_key(egui::Modifiers::NONE, egui::Key::Slash);
                    input
                        .events
                        .retain(|event| !matches!(event, egui::Event::Text(text) if text == "/"));
                });
            }
            ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("global-search")));
            self.search_suggestions_open =
                !self.query.trim().is_empty() || !self.search_history.is_empty();
        }
        if shortcuts.4 {
            self.page = Page::Home;
        }
        if shortcuts.5 {
            self.page = Page::Songs;
        }
        if shortcuts.6 {
            self.page = Page::Albums;
        }
        if shortcuts.7 {
            self.page = Page::Artists;
        }
        if shortcuts.8 {
            self.page = Page::LikedSongs;
        }
        if shortcuts.9 {
            self.page = Page::Playlists;
        }
        if shortcuts.10 {
            self.page = Page::History;
        }
        if shortcuts.11 {
            self.page = Page::Mixes;
        }
        if shortcuts.12 {
            self.page = Page::Settings;
        }
        if shortcuts.13 {
            self.start_scan();
        }
        if shortcuts.14 {
            self.add_folder();
        }
        if shortcuts.15 {
            if let Some(index) = self
                .current
                .and_then(|position| self.queue.get(position))
                .copied()
            {
                self.toggle_song_reaction(index, true);
            }
        }
        if shortcuts.16 {
            self.queue_collapsed = !self.queue_collapsed;
        }
        if shortcuts.17 && self.search_suggestions_open {
            self.search_suggestions_open = false;
        }
        if shortcuts.18 {
            self.volume = (self.volume + 0.05).min(1.0);
            if let Some(sink) = &self.sink {
                    sink.set_volume(self.current_output_volume());
            }
            self.save_state();
        }
        if shortcuts.19 {
            self.volume = (self.volume - 0.05).max(0.0);
            if let Some(sink) = &self.sink {
                    sink.set_volume(self.current_output_volume());
            }
            self.save_state();
        }
        if self.last_animated_page != self.page {
            self.last_animated_page = self.page;
            self.page_animation_started = Instant::now();
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
            let fade_window = Duration::from_secs_f32(self.track_fade_seconds.clamp(0.0, 8.0));
            let until_fade = until_end.saturating_sub(fade_window);
            let delay = if fade_window.is_zero() {
                until_end
            } else if until_fade.is_zero() {
                Duration::from_millis(100)
            } else {
                until_fade
            };
            ctx.request_repaint_after(delay);
        } else if window_focused && (self.scan_busy || self.duplicate_scan_busy) {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        ctx.set_visuals(egui::Visuals::dark());
        let accent = accent_color(&self.accent);
        ctx.style_mut(|style| {
            style.animation_time = if self.animations_enabled { 0.16 } else { 0.0 };
            style.visuals.panel_fill = Color32::from_rgb(13, 14, 20);
            style.visuals.window_fill = Color32::from_rgb(22, 24, 33);
            style.visuals.extreme_bg_color = Color32::from_rgb(17, 18, 25);
            style.visuals.faint_bg_color = Color32::from_rgb(26, 28, 38);
            style.visuals.selection.bg_fill = accent;
            style.visuals.widgets.active.bg_fill = accent;
            style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(42, 48, 76);
            style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(25, 26, 36);
            style.visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(16, 18, 25);
            style.visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(22, 23, 32);
            style.visuals.widgets.noninteractive.corner_radius = egui::CornerRadius::same(12);
            style.visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(18);
            style.visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(18);
            style.visuals.widgets.active.corner_radius = egui::CornerRadius::same(18);
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
                .insert(egui::TextStyle::Body, egui::FontId::proportional(16.0));
            style
                .text_styles
                .insert(egui::TextStyle::Button, egui::FontId::proportional(15.0));
            style
                .text_styles
                .insert(egui::TextStyle::Small, egui::FontId::proportional(13.0));
            style
                .text_styles
                .insert(egui::TextStyle::Heading, egui::FontId::proportional(30.0));
        });

        let splash_elapsed = self.splash_started.elapsed().as_secs_f32();
        if splash_elapsed < 0.5 {
            if window_focused {
                ctx.request_repaint_after(Duration::from_millis(16));
            }
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(splash_background(splash_elapsed)))
                .show(ctx, |ui| {
                    ui.centered_and_justified(|ui| {
                        ui.label(
                            RichText::new("SupremyTommy")
                                .size(34.0)
                                .strong()
                                .color(Color32::WHITE),
                        );
                    });
                });
            return;
        }

        if self.welcome_open {
            if window_focused {
                ctx.request_repaint_after(Duration::from_millis(120));
            }
            self.show_first_run_welcome(ctx);
            show_splash_overlay(ctx, splash_elapsed);
            show_custom_window_resize_grips(ctx);
            return;
        }

        if self.initial_library_loading {
            if window_focused {
                ctx.request_repaint_after(Duration::from_millis(160));
            }
            let loading_top_bar = egui::TopBottomPanel::top("top-bar")
                .frame(
                    egui::Frame::new()
                        .fill(Color32::from_rgb(14, 15, 22))
                        .inner_margin(egui::Margin::symmetric(12, 5)),
                )
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.heading(
                            RichText::new("♫ LILT  ·  SupremyTommy")
                                .color(accent)
                                .strong()
                                .size(20.0),
                        );
                        let drag = ui.add_sized(
                            [ui.available_width() - 150.0, 34.0],
                            egui::Label::new("").sense(egui::Sense::click_and_drag()),
                        );
                        if drag.drag_started() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                        }
                        if drag.double_clicked() {
                            let maximized =
                                ctx.input(|input| input.viewport().maximized.unwrap_or(false));
                            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                        }
                        show_custom_window_controls(ui, ctx);
                    });
                });
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
            show_splash_overlay(ctx, splash_elapsed);
            show_custom_window_resize_grips(ctx);
            ctx.move_to_top(loading_top_bar.response.layer_id);
            return;
        }

        let mut search_rect = None;
        let mut search_focused = false;
        let mut search_entered = false;
        let top_bar = egui::TopBottomPanel::top("top-bar")
            .exact_height(48.0)
            .frame(egui::Frame::new().fill(Color32::from_rgb(22, 24, 33)))
            .show(ctx, |ui| {
                ui.set_min_height(48.0);
                egui::Frame::new()
                    .fill(Color32::from_rgb(22, 24, 33))
                    .stroke(egui::Stroke::new(1.0_f32, Color32::from_rgb(38, 42, 55)))
                    .inner_margin(egui::Margin::symmetric(14, 6))
                    .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let drag = ui.add_sized(
                        [230.0, 34.0],
                        egui::Label::new(
                            RichText::new("♫ LILT  ·  SupremyTommy")
                                .color(accent)
                                .strong()
                                .size(20.0),
                        )
                        .sense(egui::Sense::click_and_drag()),
                    );
                    if drag.drag_started() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                    }
                    if drag.double_clicked() {
                        let maximized =
                            ctx.input(|input| input.viewport().maximized.unwrap_or(false));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                    }
                    if ui.available_width() > 1220.0 {
                        ui.menu_button("File", |ui| self.show_topbar_file_menu(ui));
                        ui.menu_button("Playback", |ui| self.show_topbar_playback_menu(ui));
                        ui.menu_button("View", |ui| self.show_topbar_view_menu(ui));
                        ui.menu_button("Help", |ui| self.show_topbar_help_menu(ui));
                    } else {
                        ui.menu_button("☰ Menu", |ui| {
                            ui.menu_button("File", |ui| self.show_topbar_file_menu(ui));
                            ui.menu_button("Playback", |ui| self.show_topbar_playback_menu(ui));
                            ui.menu_button("View", |ui| self.show_topbar_view_menu(ui));
                            ui.menu_button("Help", |ui| self.show_topbar_help_menu(ui));
                        });
                    }
                    ui.add_space(4.0);
                    if let Some(icon) = self.ui_icons.get("search") {
                        ui.add(egui::Image::new(icon).fit_to_exact_size(Vec2::splat(18.0)));
                    }
                    let search_width = ui.available_width().clamp(170.0, 300.0);
                    let search_response = ui.add_sized(
                        [search_width, 34.0],
                        egui::TextEdit::singleline(&mut self.query)
                            .id(egui::Id::new("global-search"))
                            .hint_text("Search your library…"),
                    );
                    if search_response.changed() {
                        self.search_suggestions_open =
                            !self.query.trim().is_empty() || !self.search_history.is_empty();
                        self.search_suggestion_selection = 0;
                        self.search_suggestions_focused = false;
                        self.search_ai_results.clear();
                        self.search_committed = false;
                    }
                    if search_response.clicked() || search_response.gained_focus() {
                        // Clicking or tabbing back into the textbox exits suggestion
                        // navigation so the first result is never implicitly selected.
                        self.search_suggestions_focused = false;
                        self.search_suggestions_open =
                            !self.query.trim().is_empty() || !self.search_history.is_empty();
                    }
                    search_focused = search_response.has_focus();
                    search_entered = search_focused
                        && ctx.input(|input| input.key_pressed(egui::Key::Enter))
                        && !self.query.trim().is_empty();
                    if search_entered {
                        // Reserve Enter in the textbox for the complete results view.
                        // The live suggestion popup also listens for Enter to activate
                        // its highlighted row, so consume it before that popup is drawn.
                        ctx.input_mut(|input| {
                            input.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
                        });
                        self.search_suggestions_focused = false;
                    }
                    if self.search_suggestions_focused {
                        search_response.surrender_focus();
                    }
                    search_rect = Some(search_response.rect);
                    if icon_text_button(
                        ui,
                        self.ui_icons.get("queue"),
                        format!("Queue · {} total", self.queue.len()),
                        !self.queue_collapsed,
                    )
                        .on_hover_text("Total tracks in the current playback queue")
                        .clicked()
                    {
                        self.queue_collapsed = !self.queue_collapsed;
                        self.save_state();
                    }
                    if icon_text_button(
                        ui,
                        self.ui_icons.get("settings"),
                        "Settings",
                        self.page == Page::Settings,
                    )
                        .clicked()
                    {
                        self.page = Page::Settings;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_sized([42.0, 34.0], egui::Button::new("×"))
                            .on_hover_text("Close")
                            .clicked()
                        {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        let maximized =
                            ctx.input(|input| input.viewport().maximized.unwrap_or(false));
                        if add_window_state_button(ui, maximized)
                            .on_hover_text(if maximized { "Restore" } else { "Maximize" })
                            .clicked()
                        {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                        }
                        if ui
                            .add_sized([42.0, 34.0], egui::Button::new("—"))
                            .on_hover_text("Minimize")
                            .clicked()
                        {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                        }
                        ui.separator();
                        if self.scan_busy || self.online_artwork_pending {
                            ui.spinner();
                            let pulse_phase = ctx.input(|input| input.time) as f32;
                            let pulse = 0.62 + (pulse_phase * 3.0).sin().abs() * 0.38;
                            ui.label(
                                RichText::new(if self.online_artwork_pending { &self.online_artwork_status } else { &self.scan_status })
                                    .small()
                                    .color(accent.linear_multiply(pulse)),
                            );
                        } else {
                            ui.label(
                                RichText::new(if self.online_artwork_status.is_empty() { &self.scan_status } else { &self.online_artwork_status })
                                    .small()
                                    .color(Color32::GRAY),
                            );
                        }
                    });
                });

                    });
            });

        if let Some(search_rect) = search_rect {
            self.show_search_suggestions(ctx, search_rect);
        }
        if search_entered {
            self.remember_search(&self.query.clone());
            self.search_committed = true;
            self.search_suggestions_open = false;
            self.search_suggestions_focused = false;
            self.search_suggestion_selection = 0;
            self.page = Page::Home;
        }

        let player_transition = if self.animations_enabled {
            ctx.animate_bool_with_time(
                egui::Id::new("now-playing-panel-transition"),
                self.now_playing_expanded,
                0.34,
            )
        } else if self.now_playing_expanded {
            1.0
        } else {
            0.0
        };
        if (0.001..0.999).contains(&player_transition) {
            ctx.request_repaint_after(Duration::from_millis(16));
        }

        egui::TopBottomPanel::bottom("player-controls")
            .exact_height(88.0)
            .frame(
                egui::Frame::new()
                    .fill(Color32::from_rgb(13, 14, 20))
                    .inner_margin(0),
            )
            .show(ctx, |ui| {
                ui.add_space(4.0);
                egui::Frame::new()
                    .fill(Color32::from_rgb(16, 17, 20))
                    .stroke(egui::Stroke::new(1.0_f32, Color32::from_rgb(38, 40, 46)))
                    .corner_radius(egui::CornerRadius::same(10))
                    .inner_margin(egui::Margin::symmetric(12, 4))
                    .show(ui, |ui| {
                        let player_frame_response = ui.interact(
                            ui.available_rect_before_wrap(),
                            ui.id().with("expand-player-frame"),
                            egui::Sense::click(),
                        );
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
                                    ui.allocate_exact_size(Vec2::splat(48.0), egui::Sense::hover());
                                paint_cover_or_placeholder(
                                    ui,
                                    current_cover.as_ref(),
                                    cover_rect,
                                    current_track_index.unwrap_or(0),
                                );
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
                                let play_animation = if self.animations_enabled {
                                    ctx.animate_bool_with_time(
                                        egui::Id::new("player-play-animation"),
                                        playing,
                                        0.18,
                                    )
                                } else if playing {
                                    1.0
                                } else {
                                    0.0
                                };
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
                                            egui::Image::new(&self.ui_icons["like"])
                                                .tint(if current_track_liked {
                                                    accent
                                                } else {
                                                    Color32::from_rgb(166, 171, 184)
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
                                                .tint(if current_track_disliked {
                                                    Color32::from_rgb(255, 89, 100)
                                                } else {
                                                    Color32::from_rgb(166, 171, 184)
                                                })
                                                .fit_to_exact_size(Vec2::splat(20.0)),
                                        )
                                        .frame(false),
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
                                        ui.spacing_mut().slider_rail_height = 3.5;
                                        ui.add_sized(
                                            [112.0, 22.0],
                                            egui::Slider::new(&mut self.volume, 0.0..=1.0)
                                                .show_value(false),
                                        )
                                    })
                                    .inner
                                    .on_hover_text(format!("Volume: {:.0}%", self.volume * 100.0));
                                if volume_response.changed() {
                                    if let Some(sink) = &self.sink {
                                        sink.set_volume(self.current_output_volume());
                                    }
                                    self.save_state();
                                }
                                if ui.button("Queue").clicked() {
                                    self.queue_collapsed = !self.queue_collapsed;
                                }
                            });
                            if player_frame_response.clicked() {
                                self.now_playing_expanded = !self.now_playing_expanded;
                            }
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

        let mut playlist_sidebar_action: Option<(usize, u8)> = None;
        if !self.now_playing_expanded && player_transition <= 0.001 {
            egui::SidePanel::left("navigation")
                .resizable(true)
                .default_width(228.0)
                .width_range(200.0..=300.0)
                .frame(
                    egui::Frame::new()
                        .fill(Color32::from_rgb(15, 16, 23))
                        .stroke(egui::Stroke::new(1.0_f32, Color32::from_rgb(38, 42, 55))),
                )
                .show(ctx, |ui| {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("♫").size(24.0).strong().color(accent));
                        ui.vertical(|ui| {
                            ui.label(RichText::new("Lilt Library").strong().size(16.0));
                            ui.label(
                                RichText::new("Your music space")
                                    .small()
                                    .color(Color32::GRAY),
                            );
                        });
                    });
                    ui.add_space(8.0);
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .id_salt("navigation-items")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            for (label, items) in [
                                (
                                    "LISTEN",
                                    vec![
                                        (Page::Home, "Home", "home"),
                                        (Page::Discover, "Discover", "discover"),
                                        (Page::Mixes, "Smart mixes", "smart_mixes"),
                                    ],
                                ),
                                (
                                    "LIBRARY",
                                    vec![
                                        (Page::Songs, "Songs", "songs"),
                                        (Page::Albums, "Albums", "album"),
                                        (Page::Artists, "Artists", "person"),
                                    ],
                                ),
                                (
                                    "YOUR COLLECTION",
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
                                        (Page::Stats, "Listening stats", "history"),
                                        (Page::Disliked, "Disliked", "dislike"),
                                        (Page::Lyrics, "Lyrics", "lyrics"),
                                    ],
                                ),
                            ] {
                                ui.label(
                                    RichText::new(label)
                                        .small()
                                        .strong()
                                        .color(Color32::from_rgb(135, 146, 166)),
                                );
                                ui.add_space(5.0);
                                for (page, title, icon) in items {
                                    if page == Page::Lyrics && !self.lyrics_enabled {
                                        continue;
                                    }
                                    if animated_nav_item(
                                        ui,
                                        &format!("{label}-{title}"),
                                        title,
                                        self.ui_icons.get(icon),
                                        self.page == page,
                                        accent,
                                    ).clicked() {
                                        self.page = page;
                                        self.selected_playlist = None;
                                        self.now_playing_expanded = false;
                                    }
                                }
                                ui.add_space(10.0);
                            }
                            ui.separator();
                            ui.horizontal(|ui| {
                                let playlists_heading = ui.add(
                                    egui::Label::new(
                                        RichText::new("PLAYLISTS")
                                            .small()
                                            .strong()
                                            .color(Color32::from_rgb(135, 146, 166)),
                                    )
                                    .sense(egui::Sense::click()),
                                );
                                if playlists_heading
                                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                                    .on_hover_text("Show all playlists")
                                    .clicked()
                                {
                                    self.page = Page::Playlists;
                                    self.selected_playlist = None;
                                    self.now_playing_expanded = false;
                                }
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui
                                            .add_sized(
                                                [34.0, 32.0],
                                                egui::Button::image(
                                                    egui::Image::new(&self.ui_icons["add"])
                                                        .fit_to_exact_size(Vec2::splat(18.0)),
                                                ),
                                            )
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
                                let response = animated_nav_item(
                                    ui,
                                    &format!("playlist-{index}"),
                                    playlist_name,
                                    self.ui_icons.get("library_books"),
                                    selected,
                                    accent,
                                );
                                let clicked = response.clicked();
                                response.context_menu(|ui| {
                                    ui.label(RichText::new(playlist_name).strong());
                                    ui.separator();
                                    if ui.button("Open playlist").clicked() {
                                        playlist_sidebar_action = Some((index, 0));
                                        ui.close_menu();
                                    }
                                    let has_tracks = !self.playlist_track_indices(index).is_empty();
                                    if ui.add_enabled(has_tracks, egui::Button::new("Play now")).clicked() {
                                        playlist_sidebar_action = Some((index, 1));
                                        ui.close_menu();
                                    }
                                    if ui.add_enabled(has_tracks, egui::Button::new("Shuffle and play")).clicked() {
                                        playlist_sidebar_action = Some((index, 2));
                                        ui.close_menu();
                                    }
                                    if ui.add_enabled(has_tracks, egui::Button::new("Add to queue")).clicked() {
                                        playlist_sidebar_action = Some((index, 3));
                                        ui.close_menu();
                                    }
                                    ui.separator();
                                    if ui.button("Rename").clicked() {
                                        playlist_sidebar_action = Some((index, 4));
                                        ui.close_menu();
                                    }
                                    if ui.button("Duplicate").clicked() {
                                        playlist_sidebar_action = Some((index, 5));
                                        ui.close_menu();
                                    }
                                    if ui.button("Delete playlist").clicked() {
                                        playlist_sidebar_action = Some((index, 6));
                                        ui.close_menu();
                                    }
                                });
                                if clicked {
                                    self.page = Page::Playlists;
                                    self.selected_playlist = Some(index);
                                    self.now_playing_expanded = false;
                                }
                            }
                        });
                    ui.separator();
                    if animated_nav_item(
                        ui,
                        "pinned-settings",
                        "Settings",
                        self.ui_icons.get("settings"),
                        self.page == Page::Settings,
                        accent,
                    ).clicked() {
                        self.page = Page::Settings;
                        self.now_playing_expanded = false;
                    }
                    ui.add_space(2.0);
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
        }

        if let Some((index, action)) = playlist_sidebar_action {
            if let Some(playlist) = self.playlists.get(index).cloned() {
                let tracks = self.playlist_track_indices(index);
                match action {
                    0 => {
                        self.page = Page::Playlists;
                        self.selected_playlist = Some(index);
                        self.now_playing_expanded = false;
                    }
                    1 => self.play_playlist(index, false),
                    2 => self.play_playlist(index, true),
                    3 => self.add_tracks_to_queue(&tracks),
                    4 => {
                        self.page = Page::Playlists;
                        self.selected_playlist = None;
                        self.playlist_rename_draft = playlist.name;
                        self.renaming_playlist = Some(index);
                    }
                    5 => {
                        let mut name = format!("{} Copy", playlist.name);
                        let mut suffix = 2;
                        while self.playlists.iter().any(|item| item.name.eq_ignore_ascii_case(&name)) {
                            name = format!("{} Copy {suffix}", playlist.name);
                            suffix += 1;
                        }
                        self.playlists.push(Playlist { name, ..playlist });
                        self.save_state();
                        self.status = "Playlist duplicated.".into();
                    }
                    6 => {
                        self.playlists.remove(index);
                        if let Some(selected) = self.selected_playlist {
                            self.selected_playlist = if selected == index { None } else if selected > index { Some(selected - 1) } else { Some(selected) };
                        }
                        self.save_state();
                        self.status = "Playlist deleted. Its audio files were kept.".into();
                    }
                    _ => {}
                }
            }
        }

        if !self.queue_collapsed && !self.now_playing_expanded && player_transition <= 0.001 {
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
                            self.cancel_crossfade();
                            self.queue.clear();
                            self.current = None;
                            self.sink = None;
                            self.stream = None;
                            self.resume_queue_paths.clear();
                            self.resume_current_path = None;
                            self.resume_position_seconds = 0.0;
                            self.save_state();
                        }
                    });
                    ui.horizontal(|ui| {
                        if let Some(icon) = self.ui_icons.get("shuffle") {
                            ui.add(egui::Image::new(icon).fit_to_exact_size(Vec2::splat(18.0)));
                        }
                        if ui
                            .selectable_label(self.shuffle, "Shuffle")
                            .on_hover_text("Toggle queue shuffle")
                            .clicked()
                        {
                            self.cancel_crossfade();
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
                                                ui.add(
                                                    egui::Image::new((
                                                        cover.id(),
                                                        Vec2::splat(56.0),
                                                    ))
                                                    .corner_radius(egui::CornerRadius::same(14)),
                                                );
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
                                    if icon_text_button(ui, self.ui_icons.get("next"), "Skip to next track", false).clicked() {
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
                                    if icon_text_button(ui, self.ui_icons.get("copy"), "Copy song title and artist", false).clicked() {
                                        ui.ctx().copy_text(format!("{} — {}", title, artist));
                                        ui.close_menu();
                                    }
                                    if icon_text_button(ui, self.ui_icons.get("copy"), "Copy file path", false).clicked() {
                                        ui.ctx().copy_text(current_path.to_string_lossy().into_owned());
                                        ui.close_menu();
                                    }
                                    #[cfg(target_os = "windows")]
                                    if icon_text_button(ui, self.ui_icons.get("folder_open"), "Show in File Explorer", false).clicked() {
                                        reveal_path_in_explorer(&current_path);
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
                                                ui.add(
                                                    egui::Image::new((
                                                        cover.id(),
                                                        Vec2::splat(46.0),
                                                    ))
                                                    .corner_radius(egui::CornerRadius::same(12)),
                                                );
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
                                        if icon_text_button(ui, self.ui_icons.get("copy"), "Copy song title and artist", false).clicked() {
                                            ui.ctx().copy_text(format!("{} — {}", track_title, track_artist));
                                            ui.close_menu();
                                        }
                                        if icon_text_button(ui, self.ui_icons.get("copy"), "Copy file path", false).clicked() {
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
                        self.cancel_crossfade();
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
                        self.cancel_crossfade();
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
                        self.save_state();
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

        if self.developer_output_open {
            egui::SidePanel::right("developer-output-panel")
                .resizable(true)
                .default_width(360.0)
                .min_width(250.0)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.heading("Developer Output");
                        if ui.small_button("Clear").clicked() {
                            self.developer_output.clear();
                        }
                        if ui.small_button("×").clicked() {
                            self.developer_output_open = false;
                        }
                    });
                    ui.label(
                        RichText::new("Local app status and errors · no API keys are logged")
                            .small()
                            .color(Color32::GRAY),
                    );
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .id_salt("developer-output-scroll")
                        .stick_to_bottom(true)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            if self.developer_output.is_empty() {
                                ui.label(
                                    RichText::new("Waiting for app output…").color(Color32::GRAY),
                                );
                            }
                            for entry in &self.developer_output {
                                let is_error = entry.contains("[ERROR]");
                                ui.label(RichText::new(entry).monospace().size(11.0).color(
                                    if is_error {
                                        Color32::from_rgb(255, 105, 110)
                                    } else {
                                        Color32::from_rgb(190, 196, 210)
                                    },
                                ));
                            }
                        });
                });
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(Color32::from_rgb(13, 14, 20)))
            .show(ctx, |ui| {
                let page_progress = if self.animations_enabled {
                    (self.page_animation_started.elapsed().as_secs_f32() / 0.21).clamp(0.0, 1.0)
                } else {
                    1.0
                };
                let page_ease = 1.0 - (1.0 - page_progress).powi(3);
                if page_progress < 1.0 {
                    ctx.request_repaint_after(Duration::from_millis(16));
                }
                if self.now_playing_expanded || player_transition > 0.001 {
                    ui.add_space((1.0 - player_transition) * 42.0);
                    self.show_now_playing_view(ui);
                } else {
                    ui.set_opacity(page_ease);
                    ui.add_space((1.0 - page_ease) * 14.0);
                    self.show_library(ui);
                }
            });

        if let Some((message, created)) = self.error_notification.clone() {
            if created.elapsed() < Duration::from_secs(8) {
                ctx.request_repaint_after(Duration::from_millis(250));
                let mut dismiss = false;
                egui::Area::new(egui::Id::new("error-notification"))
                    .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-18.0, 58.0))
                    .order(egui::Order::Foreground)
                    .show(ctx, |ui| {
                        egui::Frame::popup(ui.style())
                            .fill(Color32::from_rgb(48, 24, 29))
                            .stroke(egui::Stroke::new(1.0_f32, Color32::from_rgb(192, 69, 77)))
                            .inner_margin(egui::Margin::symmetric(14, 10))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.vertical(|ui| {
                                        ui.label(
                                            RichText::new("Lilt needs attention")
                                                .strong()
                                                .color(Color32::from_rgb(255, 135, 140)),
                                        );
                                        ui.label(message);
                                        if self.developer_console_enabled
                                            && ui.small_button("Open Developer Output").clicked()
                                        {
                                            self.developer_output_open = true;
                                        }
                                    });
                                    dismiss = ui.small_button("×").clicked();
                                });
                            });
                    });
                if dismiss {
                    self.error_notification = None;
                }
            } else {
                self.error_notification = None;
            }
        }

        self.show_search_ai_window(ctx);
        self.show_properties_window(ctx);

        if self.online_artwork_prompt_open {
            let mut enable = false;
            let mut skip = false;
            egui::Window::new("Find missing album covers?")
                .id(egui::Id::new("online-artwork-opt-in"))
                .collapsible(false).resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label("Lilt can look up missing cover art online after scanning your library.");
                    ui.label(RichText::new("Lookups run slowly in the background, use MusicBrainz and Cover Art Archive, and are cached on this device. Track names and artist / album metadata are sent to find a match.").small().color(Color32::GRAY));
                    ui.horizontal(|ui| {
                        enable = ui.add_sized([190.0, 38.0], egui::Button::new("Fetch in background")).clicked();
                        skip = ui.button("Not now").clicked();
                    });
                });
            if enable {
                self.fetch_online_artwork = true;
                self.online_artwork_prompt_seen = true;
                self.online_artwork_prompt_open = false;
                self.save_state();
                self.start_online_artwork_fetch();
            } else if skip {
                self.fetch_online_artwork = false;
                self.online_artwork_prompt_seen = true;
                self.online_artwork_prompt_open = false;
                self.save_state();
            }
        }

        if self.release_notes_open {
            let mut open = true;
            egui::Window::new("What’s new in Lilt")
                .id(egui::Id::new("release-notes"))
                .open(&mut open).collapsible(false).resizable(true)
                .default_width(460.0).default_height(360.0)
                .show(ctx, |ui| {
                    ui.heading(format!("Lilt Player {}", env!("CARGO_PKG_VERSION")));
                    ui.label(RichText::new("Release notes").color(Color32::GRAY));
                    ui.separator();
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        ui.heading("Search");
                        ui.label("Live search shows five mixed song, artist, and album results, with compact empty states and Up/Down navigation. Enter in the search box opens full results; Enter on a selected suggestion opens it. Ask AI opens a separate answer panel with a short explanation and matching songs after it.");
                        ui.heading("Navigation");
                        ui.label("The top bar restores individual menus when there is room and keeps them grouped on narrower windows. Keyboard shortcuts are always available; Down enters live results and Up returns to the search field.");
                        ui.heading("AI library review");
                        ui.label("Settings → AI can ask Google to flag likely duplicates and high-confidence wrong artist credits. It sends compact title, artist, and album labels only; it never changes tags automatically.");
                        ui.heading("Artwork and settings");
                        ui.label("Opt-in background lookups can fill missing album and artist images. Settings save immediately, while sliders wait for Save slider changes.");
                    });
                });
            self.release_notes_open = open;
        }

        if self.default_app_prompt_open {
            let mut open_settings = false;
            let mut not_now = false;
            egui::Window::new("Make Lilt your default music player?")
                .id(egui::Id::new("lilt-default-app-prompt"))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label("Lilt can play MP3, FLAC, WAV, M4A, OGG, OPUS, and AAC files.");
                    ui.label("Windows requires you to choose your default apps in Settings.");
                    ui.horizontal(|ui| {
                        open_settings = ui.button("Choose Lilt in Windows Settings").clicked();
                        not_now = ui.button("Not now").clicked();
                    });
                });
            if open_settings {
                self.default_app_prompt_open = false;
                self.save_state();
                match open_default_apps_settings() {
                    Ok(()) => {
                        self.update_notice = Some(
                            "In Windows Settings, choose Lilt as the default app for the music formats you want it to open.".into(),
                        );
                    }
                    Err(error) => self.update_notice = Some(error),
                }
            } else if not_now {
                self.default_app_prompt_open = false;
                self.save_state();
            }
        }

        if let Some(update) = self.update_available.clone() {
            let mut later = false;
            let mut update_now = false;
            egui::Window::new("Lilt update available")
                .id(egui::Id::new("lilt-update-available"))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label(RichText::new(format!("Lilt {} is ready to download.", update.version)).strong());
                    ui.label("If you choose Update now, Lilt will download the MSI and open it. Windows will ask you to confirm installation.");
                    if self.update_downloading {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("Downloading the installer…");
                        });
                    }
                    ui.horizontal(|ui| {
                        update_now = ui
                            .add_enabled(!self.update_downloading, egui::Button::new("Update now"))
                            .clicked();
                        later = ui.add_enabled(!self.update_downloading, egui::Button::new("Later")).clicked();
                    });
                });
            if update_now {
                self.download_update();
            } else if later {
                self.update_available = None;
            }
        }
        if let Some(notice) = self.update_notice.clone() {
            let mut dismiss = false;
            egui::Window::new("Lilt updater")
                .id(egui::Id::new("lilt-update-notice"))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label(notice);
                    dismiss = ui.button("OK").clicked();
                });
            if dismiss {
                self.update_notice = None;
            }
        }

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
        show_custom_window_resize_grips(ctx);
        show_splash_overlay(ctx, splash_elapsed);
        ctx.move_to_top(top_bar.response.layer_id);
    }
}

impl PlayerApp {
    fn finish_first_run(&mut self) {
        self.welcome_completed = true;
        self.welcome_open = false;
        self.recommendations_dirty = true;
        self.home_cache_dirty = true;
        self.save_state();
    }

    fn add_first_run_folder(&mut self) {
        let Some(folder) = rfd::FileDialog::new().pick_folder() else {
            return;
        };
        if !self.folders.contains(&folder) {
            self.folders.push(folder);
        }
        self.welcome_step = 2;
        self.initial_library_loading = true;
        self.save_state();
        self.start_scan();
    }

    fn show_first_run_welcome(&mut self, ctx: &egui::Context) {
        let accent = accent_color(&self.accent);
        egui::TopBottomPanel::top("first-run-titlebar")
            .exact_height(48.0)
            .frame(egui::Frame::new().fill(Color32::from_rgb(17, 19, 28)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let drag = ui.add_sized(
                        [230.0, 36.0],
                        egui::Label::new(
                            RichText::new("♫  LILT")
                                .size(20.0)
                                .strong()
                                .color(accent),
                        )
                        .sense(egui::Sense::click_and_drag()),
                    );
                    if drag.drag_started() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                    }
                    ui.add_space((ui.available_width() - 150.0).max(0.0));
                    show_custom_window_controls(ui, ctx);
                });
            });

        let mut finish = false;
        let mut add_folder = false;
        let mut remove_folder = None;
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(Color32::from_rgb(11, 13, 20)))
            .show(ctx, |ui| {
                let rect = ui.max_rect();
                ui.painter().circle_filled(
                    rect.right_top() + egui::vec2(-110.0, 140.0),
                    210.0,
                    accent.linear_multiply(0.055),
                );
                ui.painter().circle_filled(
                    rect.left_bottom() + egui::vec2(70.0, -80.0),
                    170.0,
                    Color32::from_rgb(75, 98, 179).linear_multiply(0.09),
                );
                egui::ScrollArea::vertical()
                    .id_salt("first-run-content")
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.add_space(24.0);
                        ui.vertical_centered(|ui| {
                            ui.set_max_width(860.0);
                            ui.label(
                                RichText::new("YOUR MUSIC, YOUR SPACE")
                                    .small()
                                    .strong()
                                    .color(accent),
                            );
                            ui.add_space(7.0);
                            ui.heading(
                                RichText::new(match self.welcome_step {
                                    0 => "A better way to be with your music.",
                                    1 => "Bring your music into Lilt.",
                                    _ => "Make it feel like yours.",
                                })
                                .size(34.0)
                                .strong(),
                            );
                            ui.label(
                                RichText::new(match self.welcome_step {
                                    0 => "A calm, personal home for the music you already love.",
                                    1 => "Choose the folders where your music lives. Lilt will scan them on this device.",
                                    _ => "Follow a few artists or genres to give your first mixes a head start. This is completely optional.",
                                })
                                .color(Color32::from_rgb(174, 181, 198)),
                            );
                            ui.add_space(18.0);
                            ui.horizontal(|ui| {
                                for (index, title) in ["Welcome", "Your library", "Your taste"].into_iter().enumerate() {
                                    let selected = self.welcome_step == index as u8;
                                    egui::Frame::new()
                                        .fill(if selected {
                                            accent.linear_multiply(0.22)
                                        } else {
                                            Color32::from_rgb(24, 27, 37)
                                        })
                                        .stroke(egui::Stroke::new(
                                            1.0_f32,
                                            if selected { accent } else { Color32::from_rgb(43, 47, 60) },
                                        ))
                                        .corner_radius(egui::CornerRadius::same(16))
                                        .inner_margin(egui::Margin::symmetric(13, 7))
                                        .show(ui, |ui| {
                                            ui.label(
                                                RichText::new(format!("{}  {title}", index + 1))
                                                    .small()
                                                    .strong()
                                                    .color(if selected { accent } else { Color32::GRAY }),
                                            );
                                        });
                                }
                            });
                            ui.add_space(18.0);

                            match self.welcome_step {
                                0 => {
                                    ui.columns(3, |columns| {
                                        for (column, (icon, title, body)) in columns.iter_mut().zip([
                                            ("♫", "Made for listening", "Your library, queue, mixes, and favorites in one focused player."),
                                            ("⌂", "Private by default", "Your music folders and listening history stay on this device."),
                                            ("✦", "AI stays optional", "Mix Studio only contacts Google when you ask it to make a mix."),
                                        ]) {
                                            egui::Frame::new()
                                                .fill(Color32::from_rgb(20, 23, 33))
                                                .stroke(egui::Stroke::new(1.0_f32, Color32::from_rgb(42, 47, 62)))
                                                .corner_radius(egui::CornerRadius::same(14))
                                                .inner_margin(egui::Margin::same(16))
                                                .show(column, |ui| {
                                                    ui.label(RichText::new(icon).size(24.0).color(accent));
                                                    ui.label(RichText::new(title).strong());
                                                    ui.label(RichText::new(body).small().color(Color32::GRAY));
                                                });
                                        }
                                    });
                                    ui.add_space(18.0);
                                    if ui.add_sized([250.0, 48.0], egui::Button::new("Get started  →").fill(accent)).clicked() {
                                        self.welcome_step = 1;
                                    }
                                }
                                1 => {
                                    egui::Frame::new()
                                        .fill(Color32::from_rgb(20, 23, 33))
                                        .stroke(egui::Stroke::new(1.0_f32, Color32::from_rgb(42, 47, 62)))
                                        .corner_radius(egui::CornerRadius::same(14))
                                        .inner_margin(egui::Margin::same(18))
                                        .show(ui, |ui| {
                                            ui.label(RichText::new("MUSIC FOLDERS").small().strong().color(accent));
                                            ui.add_space(8.0);
                                            if self.folders.is_empty() {
                                                ui.label("No folders selected yet.");
                                            } else {
                                                for (index, folder) in self.folders.iter().enumerate() {
                                                    ui.horizontal(|ui| {
                                                        ui.label(RichText::new(format!("♫  {}", folder.display())).color(Color32::from_rgb(215, 220, 232)));
                                                        if ui.add_enabled(!self.scan_busy, egui::Button::new("Remove")).clicked() {
                                                            remove_folder = Some(index);
                                                        }
                                                    });
                                                }
                                            }
                                            ui.add_space(12.0);
                                            if ui.add_enabled(!self.scan_busy, egui::Button::new("＋  Choose music folder").min_size(Vec2::new(210.0, 42.0)).fill(accent)).clicked() {
                                                add_folder = true;
                                            }
                                            if self.scan_busy {
                                                ui.horizontal(|ui| {
                                                    ui.add(egui::Spinner::new());
                                                    ui.label(RichText::new(&self.scan_status).small().color(Color32::GRAY));
                                                });
                                            } else if !self.folders.is_empty() {
                                                ui.label(RichText::new(format!("{} songs ready", self.tracks.len())).small().color(Color32::GRAY));
                                            }
                                        });
                                    ui.add_space(14.0);
                                    ui.horizontal(|ui| {
                                        if ui.button("← Back").clicked() { self.welcome_step = 0; }
                                        if ui.add_enabled(!self.folders.is_empty(), egui::Button::new("Continue  →").fill(accent)).clicked() { self.welcome_step = 2; }
                                        if ui.button("I’ll choose folders later").clicked() { self.welcome_step = 2; }
                                    });
                                }
                                _ => {
                                    if self.tracks.is_empty() {
                                        egui::Frame::new()
                                            .fill(Color32::from_rgb(34, 27, 25))
                                            .stroke(egui::Stroke::new(1.0_f32, Color32::from_rgb(112, 78, 53)))
                                            .corner_radius(egui::CornerRadius::same(12))
                                            .inner_margin(egui::Margin::same(14))
                                            .show(ui, |ui| {
                                                if self.scan_busy {
                                                    ui.horizontal(|ui| {
                                                        ui.add(egui::Spinner::new());
                                                        ui.label(RichText::new(&self.scan_status).color(Color32::from_rgb(224, 194, 151)));
                                                    });
                                                } else if self.folders.is_empty() {
                                                    ui.label(RichText::new("No music folder selected yet.").strong());
                                                    ui.label(RichText::new("Choose a folder to build your library, or finish setup and add one later in Settings.").small().color(Color32::GRAY));
                                                } else {
                                                    ui.label(RichText::new("Lilt didn’t find playable music in those folders.").strong());
                                                    ui.label(RichText::new(&self.status).small().color(Color32::from_rgb(224, 194, 151)));
                                                    ui.label(RichText::new("Check that the folder contains supported audio files and that Windows can access it. You can remove a folder, choose another, or scan again.").small().color(Color32::GRAY));
                                                }
                                                ui.add_space(8.0);
                                                ui.horizontal(|ui| {
                                                    if ui.button("Review folders").clicked() { self.welcome_step = 1; }
                                                    if !self.folders.is_empty() && ui.add_enabled(!self.scan_busy, egui::Button::new("↻ Rescan folders")).clicked() {
                                                        self.start_scan();
                                                    }
                                                    if ui.add_enabled(!self.scan_busy, egui::Button::new("＋ Choose another folder")).clicked() {
                                                        add_folder = true;
                                                    }
                                                });
                                            });
                                    } else {
                                        let mut artist_counts = HashMap::<String, usize>::new();
                                        let mut genre_counts = HashMap::<String, usize>::new();
                                        for track in &self.tracks {
                                            for artist in artist_names(&track.artist) {
                                                *artist_counts.entry(artist).or_default() += 1;
                                            }
                                            if !is_unknown(&track.genre) {
                                                *genre_counts.entry(track.genre.trim().to_owned()).or_default() += 1;
                                            }
                                        }
                                        let mut artists = artist_counts.into_iter().collect::<Vec<_>>();
                                        artists.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
                                        artists.truncate(10);
                                        let mut genres = genre_counts.into_iter().collect::<Vec<_>>();
                                        genres.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
                                        genres.truncate(12);
                                        egui::Frame::new()
                                            .fill(Color32::from_rgb(20, 23, 33))
                                            .stroke(egui::Stroke::new(1.0_f32, Color32::from_rgb(42, 47, 62)))
                                            .corner_radius(egui::CornerRadius::same(14))
                                            .inner_margin(egui::Margin::same(18))
                                            .show(ui, |ui| {
                                                ui.label(RichText::new("ARTISTS TO FOLLOW").small().strong().color(accent));
                                                ui.add_space(7.0);
                                                ui.horizontal_wrapped(|ui| {
                                                    for (artist, count) in artists {
                                                        let selected = self.liked_artists.contains(&artist.to_lowercase());
                                                        if ui.selectable_label(selected, format!("{}  {} · {}", if selected { "♥" } else { "＋" }, artist, count)).clicked() {
                                                            self.toggle_artist_like(&artist);
                                                        }
                                                    }
                                                });
                                                ui.add_space(14.0);
                                                ui.label(RichText::new("GENRES YOU ENJOY").small().strong().color(accent));
                                                ui.add_space(7.0);
                                                ui.horizontal_wrapped(|ui| {
                                                    for (genre, count) in genres {
                                                        let key = genre.to_lowercase();
                                                        let selected = self.favorite_genres.contains(&key);
                                                        if ui.selectable_label(selected, format!("{}  {} · {}", if selected { "♥" } else { "＋" }, genre, count)).clicked() {
                                                            if !self.favorite_genres.remove(&key) { self.favorite_genres.insert(key); }
                                                            self.recommendations_dirty = true;
                                                            self.save_state();
                                                        }
                                                    }
                                                });
                                            });
                                        ui.add_space(12.0);
                                        egui::Frame::new()
                                            .fill(accent.linear_multiply(0.10))
                                            .stroke(egui::Stroke::new(1.0_f32, accent.linear_multiply(0.45)))
                                            .corner_radius(egui::CornerRadius::same(12))
                                            .inner_margin(egui::Margin::same(13))
                                            .show(ui, |ui| {
                                                ui.label(RichText::new(format!("Your library is ready · {} songs", self.tracks.len())).strong().color(accent));
                                                ui.label(RichText::new("Start on Home for your daily mixes. Open Smart mixes to explore genre radios and tune your recommendations. Your follows and genre picks shape For you.").small().color(Color32::from_rgb(190, 196, 210)));
                                            });
                                        if self.status.contains("could not be processed") {
                                            ui.add_space(8.0);
                                            ui.label(RichText::new(&self.status).small().color(Color32::from_rgb(235, 174, 103)));
                                        }
                                    }
                                    ui.add_space(14.0);
                                    ui.horizontal(|ui| {
                                        if ui.button("← Back").clicked() { self.welcome_step = 1; }
                                        if !self.folders.is_empty() && ui.add_enabled(!self.scan_busy, egui::Button::new("＋ Add another folder")).clicked() { add_folder = true; }
                                        if ui.add_sized([230.0, 46.0], egui::Button::new("Finish and start listening  →").fill(accent)).clicked() { finish = true; }
                                    });
                                }
                            }
                            ui.add_space(10.0);
                            if ui.small_button("Skip setup and open Lilt").clicked() { finish = true; }
                            ui.add_space(24.0);
                            ui.label(RichText::new("You can change folders, follows, and mix preferences any time in Settings.").small().color(Color32::from_rgb(121, 128, 145)));
                        });
                    });
            });
        if add_folder {
            self.add_first_run_folder();
        }
        if let Some(index) = remove_folder {
            if index < self.folders.len() {
                self.folders.remove(index);
                self.save_state();
                self.start_scan();
            }
        }
        if finish {
            self.finish_first_run();
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

        if self.use_listening_history {
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

        for genre in &self.favorite_genres {
            *genre_affinity.entry(genre.to_lowercase()).or_default() += 3.0;
        }

        // Recommendation feedback is intentionally stronger than passive
        // listening signals, but remains soft so one click cannot erase a
        // genuine artist or genre preference.
        for (path, rating) in &self.recommendation_feedback {
            let Some(&index) = track_by_path.get(path.as_path()) else {
                continue;
            };
            let track = &self.tracks[index];
            let direction = if *rating > 0 { 1.0 } else { -1.0 };
            for artist in artist_names(&track.artist) {
                *artist_affinity.entry(artist.to_lowercase()).or_default() += 2.0 * direction;
            }
            if !is_unknown(&track.genre) {
                *genre_affinity
                    .entry(track.genre.trim().to_lowercase())
                    .or_default() += 1.25 * direction;
            }
            if !is_unknown(&track.album) {
                *album_affinity
                    .entry(track.album.trim().to_lowercase())
                    .or_default() += 0.75 * direction;
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
                let feedback_score = match self.recommendation_feedback.get(&track.path) {
                    Some(rating) if *rating > 0 => 4.0,
                    Some(_) => -5.0,
                    None => 0.0,
                };
                let engagement = if self.use_listening_history {
                    self.track_engagement.get(&track.path).copied().unwrap_or_default()
                } else {
                    TrackEngagement::default()
                };
                let plays = engagement.plays.max(1) as f32;
                let completion_rate = (engagement.completions as f32 / plays).clamp(0.0, 1.0);
                let skip_rate = (engagement.skips as f32 / plays).clamp(0.0, 1.0);
                let replay_rate = (engagement.replays as f32 / plays).clamp(0.0, 1.0);
                let listen_ratio = if track.duration > 0.0 {
                    (engagement.listened_seconds / (track.duration * plays as f64))
                        .clamp(0.0, 1.0) as f32
                } else {
                    0.0
                };
                let engagement_score = completion_rate * 2.2 + listen_ratio * 1.1 + replay_rate * 0.8
                    + (engagement.seek_backs as f32 / plays).clamp(0.0, 1.0) * 0.35
                    - skip_rate * 2.8;
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
                        + feedback_score
                        + engagement_score
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


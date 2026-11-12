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
        assert_eq!(preferences.smart_queue_mode, SmartQueueMode::Off);
        assert!(preferences.use_listening_history);
        assert!(preferences.session_queue.is_empty());
    }

    #[test]
    fn older_manual_playlists_load_without_a_smart_rule() {
        let preferences: Preferences = serde_json::from_str(
            r#"{"playlists":[{"name":"Old playlist","tracks":["C:/Music/song.mp3"]}]}"#,
        ).expect("old playlist settings parse");
        assert_eq!(preferences.playlists.len(), 1);
        assert!(preferences.playlists[0].rule.is_none());
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
    fn playlist_play_actions_resolve_tracks_and_explain_empty_playlists() {
        let first = PathBuf::from("first.mp3");
        let second = PathBuf::from("second.mp3");
        let mut app = PlayerApp::new();
        app.tracks = vec![
            Track { path: first.clone(), ..Track::default() },
            Track { path: second.clone(), ..Track::default() },
        ];
        app.playlists = vec![
            Playlist { name: "Ordered list".into(), tracks: vec![second.clone(), first.clone()], rule: None },
            Playlist { name: "Empty list".into(), tracks: Vec::new(), rule: None },
        ];

        assert_eq!(app.select_playlist_for_playback(0), [1, 0]);
        assert!(matches!(app.page, Page::Playlists));
        assert_eq!(app.selected_playlist, Some(0));

        app.queue = vec![1];
        assert!(app.select_playlist_for_playback(1).is_empty());
        assert_eq!(app.queue, [1]);
        assert!(app.status.contains("Empty list has no available tracks"));
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
    fn recommendation_feedback_changes_track_and_artist_ranking() {
        let mut app = PlayerApp::new();
        app.tracks = vec![
            Track { path: PathBuf::from("neutral.mp3"), artist: "Neutral Artist".into(), ..Track::default() },
            Track { path: PathBuf::from("feedback.mp3"), artist: "Feedback Artist".into(), ..Track::default() },
            Track { path: PathBuf::from("related.mp3"), artist: "Feedback Artist".into(), ..Track::default() },
        ];
        app.prefer_unplayed = false;
        app.recommendation_feedback.insert(PathBuf::from("feedback.mp3"), 1);
        let more = app.compute_recommendations();
        assert_eq!(more.first(), Some(&1));
        assert!(more.iter().position(|&index| index == 2).unwrap() < more.iter().position(|&index| index == 0).unwrap());

        app.recommendation_feedback.insert(PathBuf::from("feedback.mp3"), -1);
        let less = app.compute_recommendations();
        assert_eq!(less.first(), Some(&0));
        assert!(less.iter().position(|&index| index == 2).unwrap() < less.iter().position(|&index| index == 1).unwrap());
    }

    #[test]
    fn listening_history_toggle_removes_history_from_recommendation_scores() {
        let mut app = PlayerApp::new();
        app.tracks = vec![
            Track { path: PathBuf::from("played.mp3"), artist: "Favorite Artist".into(), ..Track::default() },
            Track { path: PathBuf::from("related.mp3"), artist: "Favorite Artist".into(), ..Track::default() },
            Track { path: PathBuf::from("other.mp3"), artist: "Other Artist".into(), ..Track::default() },
        ];
        app.prefer_unplayed = false;
        app.history = vec![PathBuf::from("played.mp3")];
        let history_based = app.compute_recommendations();
        app.use_listening_history = false;
        let history_disabled = app.compute_recommendations();
        app.history.clear();
        assert_eq!(history_disabled, app.compute_recommendations());
        assert_ne!(history_based, history_disabled);
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
            smart_queue_mode: SmartQueueMode::KeepThreeReady,
            use_listening_history: false,
            session_queue: vec![PathBuf::from("queued.mp3")],
            session_current: Some(PathBuf::from("queued.mp3")),
            session_position_seconds: 42.0,
            track_fade_seconds: 3.5,
            replay_gain_mode: ReplayGainMode::Album,
            ..Preferences::default()
        };
        let encoded = serde_json::to_string(&preferences).expect("settings serialize");
        let decoded: Preferences = serde_json::from_str(&encoded).expect("settings deserialize");

        assert_eq!(decoded.discovery, 0.8);
        assert!(!decoded.prefer_unplayed);
        assert_eq!(decoded.recommendation_variety, 0.25);
        assert_eq!(decoded.recommendation_size, 80);
        assert_eq!(decoded.smart_queue_mode, SmartQueueMode::KeepThreeReady);
        assert!(!decoded.use_listening_history);
        assert_eq!(decoded.session_queue, [PathBuf::from("queued.mp3")]);
        assert_eq!(decoded.session_current, Some(PathBuf::from("queued.mp3")));
        assert_eq!(decoded.session_position_seconds, 42.0);
        assert_eq!(decoded.track_fade_seconds, 3.5);
        assert_eq!(decoded.replay_gain_mode, ReplayGainMode::Album);
    }

    #[test]
    fn restored_queue_snapshot_keeps_saved_position_before_playback_resumes() {
        let mut app = PlayerApp::new();
        app.resume_queue_paths = vec![PathBuf::from("one.mp3"), PathBuf::from("two.mp3")];
        app.resume_current_path = Some(PathBuf::from("two.mp3"));
        app.resume_position_seconds = 87.5;
        app.tracks = vec![
            Track { path: PathBuf::from("one.mp3"), ..Track::default() },
            Track { path: PathBuf::from("two.mp3"), ..Track::default() },
        ];
        app.queue = vec![0, 1];
        app.current = Some(1);

        let snapshot = app.preferences_snapshot();

        assert_eq!(snapshot.session_queue, app.resume_queue_paths);
        assert_eq!(snapshot.session_current, Some(PathBuf::from("two.mp3")));
        assert_eq!(snapshot.session_position_seconds, 87.5);
    }

    #[test]
    fn taste_profile_backup_contains_preferences_without_local_paths_or_history() {
        let profile = TasteProfileBackup {
            version: 1,
            favorite_genres: vec!["ambient".into()],
            liked_artists: vec!["Artist".into()],
            liked_albums: vec!["Album".into()],
            discovery: 0.7,
            prefer_unplayed: true,
            recommendation_variety: 0.6,
            recommendation_size: 40,
            use_listening_history: false,
        };
        let json = serde_json::to_string(&profile).expect("profile serializes");
        let restored: TasteProfileBackup = serde_json::from_str(&json).expect("profile parses");
        assert_eq!(restored.liked_artists, ["Artist"]);
        assert!(!json.contains("\"history\""));
        assert!(!json.contains("\"path\""));
    }

    #[test]
    fn track_edge_fades_are_smooth_and_bounded_for_short_tracks() {
        assert_eq!(playback_fade_gain(0.0, Some(100.0), 4.0), 0.0);
        assert_eq!(playback_fade_gain(2.0, Some(100.0), 4.0), 0.5);
        assert_eq!(playback_fade_gain(50.0, Some(100.0), 4.0), 1.0);
        assert_eq!(playback_fade_gain(98.0, Some(100.0), 4.0), 0.5);
        assert_eq!(playback_fade_gain(100.0, Some(100.0), 4.0), 0.0);
        assert_eq!(playback_fade_gain(2.0, Some(4.0), 8.0), 1.0);
        assert_eq!(playback_fade_gain(2.0, None, 0.0), 1.0);
    }

    #[test]
    fn overlapping_crossfade_uses_equal_power_gains() {
        let (start_out, start_in) = crossfade_gains(0.0);
        assert!((start_out - 1.0).abs() < f32::EPSILON);
        assert!(start_in.abs() < f32::EPSILON);

        let (mid_out, mid_in) = crossfade_gains(0.5);
        assert!((mid_out - 2.0_f32.sqrt() / 2.0).abs() < 0.0001);
        assert!((mid_in - 2.0_f32.sqrt() / 2.0).abs() < 0.0001);
        assert!((mid_out.powi(2) + mid_in.powi(2) - 1.0).abs() < 0.0001);

        let (end_out, end_in) = crossfade_gains(1.0);
        assert!(end_out.abs() < f32::EPSILON);
        assert!((end_in - 1.0).abs() < f32::EPSILON);
        assert_eq!(crossfade_gains(2.0), (end_out, end_in));
    }

    #[test]
    fn replaygain_conversion_is_bounded_and_uses_neutral_gain_when_missing() {
        assert_eq!(replay_gain_multiplier_from_db(None), 1.0);
        assert!((replay_gain_multiplier_from_db(Some(-6.0)) - 0.5011872).abs() < 0.0001);
        assert_eq!(replay_gain_multiplier_from_db(Some(80.0)), replay_gain_multiplier_from_db(Some(6.0)));
        assert_eq!(replay_gain_multiplier_from_db(Some(f32::NAN)), 1.0);
        assert_eq!(parse_replay_gain_db("-7.35 dB"), Some(-7.35));
        assert_eq!(parse_replay_gain_db("not a gain"), None);
    }

    #[test]
    fn rule_playlists_recompute_from_current_followed_artists() {
        let mut app = PlayerApp::new();
        app.tracks = vec![
            Track { path: PathBuf::from("one.mp3"), artist: "One".into(), ..Track::default() },
            Track { path: PathBuf::from("two.mp3"), artist: "Two".into(), ..Track::default() },
        ];
        app.playlists = vec![Playlist {
            name: "Followed artists Mix".into(),
            tracks: Vec::new(),
            rule: Some(SmartPlaylistRule::FollowedArtists),
        }];
        app.liked_artists.insert("one".into());
        assert_eq!(app.playlist_track_indices(0), [0]);
        app.liked_artists.insert("two".into());
        assert_eq!(app.playlist_track_indices(0), [0, 1]);
    }

    #[test]
    fn ai_mix_candidate_preview_and_request_scope_share_the_same_privacy_filters() {
        let mut app = PlayerApp::new();
        app.tracks = vec![
            Track { path: PathBuf::from("liked.mp3"), title: "Liked title".into(), artist: "Artist".into(), ..Track::default() },
            Track { path: PathBuf::from("played.mp3"), title: "Played title".into(), artist: "Artist".into(), ..Track::default() },
            Track { path: PathBuf::from("new.mp3"), title: "New title".into(), artist: "Artist".into(), ..Track::default() },
            Track { path: PathBuf::from("excluded.mp3"), title: "Excluded".into(), ..Track::default() },
        ];
        app.liked_songs.insert(PathBuf::from("liked.mp3"));
        app.history.push(PathBuf::from("played.mp3"));
        app.disliked_songs.insert(PathBuf::from("new.mp3"));
        app.excluded_tracks.insert(PathBuf::from("excluded.mp3"));
        app.ai_mix_include_liked = true;
        app.ai_mix_include_played = false;
        app.ai_mix_include_unplayed = false;

        let candidates = app.ai_mix_candidates();

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].title, "Liked title");
        assert_eq!(candidates[0].familiarity, "L");
        assert_eq!(candidates[0].artist, "Artist");
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

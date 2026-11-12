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
                let mut online_cover_cache =
                    HashMap::<(String, String), Option<Arc<Vec<u8>>>>::new();
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
                    local.push(read_track(
                        path.clone(),
                        &mut artwork_cache,
                        &mut online_cover_cache,
                    ));
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
        issue_count: errors.len(),
        issues: errors.iter().take(200).cloned().collect(),
        auto_deleted_count,
        auto_delete_failures,
        auto_deleted_paths,
    }
}

fn read_track(
    path: PathBuf,
    artwork_cache: &mut HashMap<u64, Vec<Arc<Vec<u8>>>>,
    online_cover_cache: &mut HashMap<(String, String), Option<Arc<Vec<u8>>>>,
) -> Track {
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
            track.replay_gain_track_db = tag.get_string(&ItemKey::ReplayGainTrackGain).and_then(parse_replay_gain_db);
            track.replay_gain_album_db = tag.get_string(&ItemKey::ReplayGainAlbumGain).and_then(parse_replay_gain_db);
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
    if track.artwork.is_none() && !is_unknown(&track.album) && !is_unknown(&track.artist) {
        let key = (track.artist.to_lowercase(), track.album.to_lowercase());
        let cached = online_cover_cache.entry(key).or_insert_with(|| {
            let path = online_artwork_cache_path(&track.artist, &track.album).ok()?;
            let bytes = fs::read(path).ok()?;
            if bytes.is_empty() || bytes.len() > MAX_CACHED_ARTWORK_BYTES {
                return None;
            }
            Some(Arc::new(bytes))
        });
        track.artwork = cached.clone();
    }
    track.search_text = format!(
        "{} {} {} {}",
        track.title, track.artist, track.album, track.genre
    )
    .to_lowercase();
    track
}

fn parse_replay_gain_db(value: &str) -> Option<f32> {
    let normalized = value.trim().strip_suffix("dB").unwrap_or(value).trim();
    normalized.parse::<f32>().ok().filter(|gain| gain.is_finite())
}

fn online_artwork_cache_path(artist: &str, album: &str) -> Result<PathBuf, String> {
    let project = ProjectDirs::from("com", "Lilt", "Lilt Player")
        .ok_or_else(|| "Could not locate the local album-art cache directory.".to_string())?;
    let mut hasher = DefaultHasher::new();
    artist.trim().to_lowercase().hash(&mut hasher);
    album.trim().to_lowercase().hash(&mut hasher);
    Ok(project
        .data_local_dir()
        .join("album-art")
        .join(format!("{:016x}.jpg", hasher.finish())))
}

fn artist_art_cache_key(artist: &str) -> String {
    let mut hasher = DefaultHasher::new();
    artist.trim().to_lowercase().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn online_artist_art_cache_path(artist: &str) -> Result<PathBuf, String> {
    let project = ProjectDirs::from("com", "Lilt", "Lilt Player")
        .ok_or_else(|| "Could not locate the local artist-art cache directory.".to_string())?;
    Ok(project
        .data_local_dir()
        .join("artist-art")
        .join(format!("{}.jpg", artist_art_cache_key(artist))))
}

fn artist_art_texture_key(artist: &str) -> PathBuf {
    PathBuf::from("__lilt_artist_art__").join(format!("{}.jpg", artist_art_cache_key(artist)))
}

#[cfg(windows)]
fn lookup_online_album_art(artist: &str, album: &str) -> Result<Vec<u8>, String> {
    let query = format!(
        "releasegroup:\"{}\" AND artist:\"{}\"",
        album.trim(),
        artist.trim()
    );
    let search_path = format!(
        "/ws/2/release-group/?query={}&fmt=json&limit=1",
        url_encode_component(&query)
    );
    let response = winhttp_get(
        "musicbrainz.org",
        &search_path,
        "application/json",
        512 * 1024,
    )?;
    let value: serde_json::Value = serde_json::from_slice(&response)
        .map_err(|_| "MusicBrainz returned invalid album metadata.".to_string())?;
    let id = value
        .pointer("/release-groups/0/id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "No matching album cover was found.".to_string())?;
    winhttp_get(
        "coverartarchive.org",
        &format!("/release-group/{id}/front-250"),
        "image/jpeg,image/png,image/webp",
        MAX_CACHED_ARTWORK_BYTES,
    )
}

#[cfg(windows)]
fn lookup_online_artist_art(artist: &str) -> Result<Vec<u8>, String> {
    let path = format!("/w/api.php?action=query&format=json&formatversion=2&generator=search&gsrsearch={}&gsrnamespace=0&gsrlimit=8&prop=pageimages&piprop=thumbnail&pithumbsize=400&pilimit=1", url_encode_component(artist.trim()));
    let response = winhttp_get("en.wikipedia.org", &path, "application/json", 512 * 1024)?;
    let value: serde_json::Value = serde_json::from_slice(&response)
        .map_err(|_| "Wikipedia returned invalid artist image metadata.".to_string())?;
    let pages = value
        .pointer("/query/pages")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "Wikipedia found no artist pages.".to_string())?;
    let normalized_artist = artist.trim().to_lowercase();
    let unsuitable = [
        "album",
        "discography",
        "song",
        "single",
        "soundtrack",
        "film",
        "record label",
    ];
    let page = pages
        .iter()
        .find(|page| {
            page.get("title")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|title| title.eq_ignore_ascii_case(artist.trim()))
                && page
                    .pointer("/thumbnail/source")
                    .and_then(serde_json::Value::as_str)
                    .is_some()
        })
        .or_else(|| {
            pages.iter().find(|page| {
                let Some(title) = page.get("title").and_then(serde_json::Value::as_str) else {
                    return false;
                };
                let title = title.to_lowercase();
                title.contains(&normalized_artist)
                    && !unsuitable.iter().any(|word| title.contains(word))
                    && page
                        .pointer("/thumbnail/source")
                        .and_then(serde_json::Value::as_str)
                        .is_some()
            })
        })
        .ok_or_else(|| "No matching artist portrait was found.".to_string())?;
    let source = page
        .pointer("/thumbnail/source")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "No artist image was found.".to_string())?;
    let source = source
        .strip_prefix("https://")
        .ok_or_else(|| "The artist image URL was not secure.".to_string())?;
    let (host, path) = source
        .split_once('/')
        .ok_or_else(|| "The artist image URL was invalid.".to_string())?;
    if !host.ends_with("wikimedia.org") {
        return Err("Wikipedia returned an unexpected artist image host.".into());
    }
    winhttp_get(
        host,
        &format!("/{path}"),
        "image/jpeg,image/png",
        MAX_CACHED_ARTWORK_BYTES,
    )
}

fn url_encode_component(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
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

const GOOGLE_AI_KEY_TARGET: &str = "Lilt.GoogleAI.ApiKey";
const LILT_USER_AGENT: &str = concat!(
    "LiltPlayer/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/SupremyTommy/Lilt)"
);

fn compact_metadata(value: &str, limit: usize) -> String {
    value
        .chars()
        .filter(|ch| !ch.is_control())
        .take(limit)
        .collect()
}

fn compact_tsv_field(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if matches!(ch, '\t' | '\r' | '\n' | '|') {
                ' '
            } else {
                ch
            }
        })
        .collect()
}

fn generate_google_ai_audit(
    key: &str,
    catalog: &str,
    check_duplicates: bool,
    check_artist_mismatches: bool,
    model: GeminiModel,
    thinking: GeminiThinking,
) -> Result<(Vec<AiAuditFinding>, AiUsage), String> {
    let mut tasks = Vec::new();
    if check_duplicates {
        tasks.push("find only high-confidence duplicate recordings (same recording represented more than once; do not group covers, live versions, remixes, or different songs with common titles)");
    }
    if check_artist_mismatches {
        tasks.push("find only obvious, high-confidence wrong artist credits using general music knowledge; for example Believer credited to an unrelated personal name instead of Imagine Dragons");
    }
    let prompt = format!(
        "Review this user's local music metadata. {}. Return valid JSON only using {{\"findings\":[...]}}. A duplicate finding must use kind=\"duplicate\", ids=[two or more row IDs], explanation. An artist mismatch must use kind=\"artist_mismatch\", id, suggested_artist, explanation. Include only very high-confidence findings; do not guess on obscure songs, ambiguous titles, features, aliases, or legitimate alternate versions. Return an empty findings list when unsure. Never suggest editing files. Catalog columns: id, title, credited artist, album.\n{}",
        tasks.join("; "), catalog
    );
    let body = serde_json::json!({
        "contents": [{"parts": [{"text": prompt}]}],
        "generationConfig": {
            "responseMimeType": "application/json",
            "thinkingConfig": {"thinkingLevel": thinking.id()},
            "maxOutputTokens": 2048
        }
    });
    let response = google_ai_post(key, &body.to_string(), model)?;
    let value: serde_json::Value = serde_json::from_str(&response)
        .map_err(|_| "Google returned an unreadable library review. Try again.".to_string())?;
    if let Some(message) = value
        .pointer("/error/message")
        .and_then(serde_json::Value::as_str)
    {
        return Err(format!(
            "Google AI error: {}",
            compact_metadata(message, 240)
        ));
    }
    let usage_metadata = value
        .get("usageMetadata")
        .unwrap_or(&serde_json::Value::Null);
    let usage = AiUsage {
        prompt_tokens: usage_metadata
            .get("promptTokenCount")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        output_tokens: usage_metadata
            .get("candidatesTokenCount")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        thinking_tokens: usage_metadata
            .get("thoughtsTokenCount")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        total_tokens: usage_metadata
            .get("totalTokenCount")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
    };
    let text = value
        .pointer("/candidates/0/content/parts")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|part| part.get("text").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    let cleaned = text
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    let json_text = if cleaned.starts_with('{') && cleaned.ends_with('}') {
        cleaned
    } else if let (Some(start), Some(end)) = (cleaned.find('{'), cleaned.rfind('}')) {
        &cleaned[start..=end]
    } else {
        return Err("Google returned no library review results.".into());
    };
    let parsed: serde_json::Value = serde_json::from_str(json_text)
        .map_err(|_| "Google's library review could not be read.".to_string())?;
    let known_ids = catalog
        .lines()
        .filter_map(|line| line.split('\t').next()?.parse::<usize>().ok())
        .collect::<HashSet<_>>();
    let mut findings = Vec::new();
    for finding in parsed
        .get("findings")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
    {
        let kind = finding
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let mut track_ids = if kind == "duplicate" {
            finding
                .get("ids")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(serde_json::Value::as_u64)
                .map(|id| id as usize)
                .filter(|id| known_ids.contains(id))
                .collect::<Vec<_>>()
        } else {
            finding
                .get("id")
                .and_then(serde_json::Value::as_u64)
                .map(|id| id as usize)
                .filter(|id| known_ids.contains(id))
                .into_iter()
                .collect::<Vec<_>>()
        };
        track_ids.sort_unstable();
        track_ids.dedup();
        if (kind == "duplicate" && track_ids.len() < 2)
            || (kind == "artist_mismatch" && track_ids.len() != 1)
            || !matches!(kind, "duplicate" | "artist_mismatch")
        {
            continue;
        }
        findings.push(AiAuditFinding {
            kind: kind.to_string(),
            track_ids,
            suggested_artist: finding
                .get("suggested_artist")
                .and_then(serde_json::Value::as_str)
                .map(|s| compact_metadata(s, 100))
                .unwrap_or_default(),
            explanation: finding
                .get("explanation")
                .and_then(serde_json::Value::as_str)
                .map(|s| compact_metadata(s, 320))
                .unwrap_or_else(|| "Review this suggestion.".into()),
        });
        if findings.len() >= 40 {
            break;
        }
    }
    Ok((findings, usage))
}

fn generate_google_ai_mix(
    key: &str,
    theme: &str,
    candidates: &[AiMixCandidate],
    model: GeminiModel,
    thinking: GeminiThinking,
    mix_size: usize,
    include_answer: bool,
) -> Result<(Vec<usize>, AiUsage, Option<String>), String> {
    let mix_size = if include_answer {
        5
    } else {
        mix_size.clamp(50, 200)
    };
    let catalog = candidates
        .iter()
        .enumerate()
        .map(|(id, candidate)| {
            // Compact TSV reduces prompt overhead while retaining every local
            // song's searchable title, artist, and genre metadata.
            format!(
                "{id}\t{}\t{}\t{}\t{}\t{}",
                compact_tsv_field(&candidate.title),
                compact_tsv_field(&candidate.artist),
                compact_tsv_field(&candidate.genre),
                candidate.familiarity,
                candidate.engagement
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let prompt = if include_answer {
        format!("Answer the user's music question in one or two friendly, specific sentences using only the request and catalog below. Do not claim to know anything about the user beyond these local song labels and explicit local listening signals. Then choose up to five relevant songs from the catalog. Match named artists and genres strictly; do not substitute unrelated tracks. When several tracks fit, prefer liked songs, followed artists, saved albums, high completion/replay/backward-seek signals, and avoid tracks with repeated early skips. Still include new, related discoveries where the request allows. Return valid JSON only with keys `answer` (string) and `ids` (array of catalog row IDs). If no local tracks match, explain that clearly and return an empty ids array. Request: {}\nCatalog columns: id, title, artist, genre, familiarity (L=liked, P=played, N=new), behavior flags (l=liked, f=followed artist, a=saved album, h=heard percent, p=plays, c=completions, s=early skips, r=replays, b=backward seeks).\n{}", compact_metadata(theme, 120), catalog)
    } else {
        format!("Build a music mix for this request: {}. Choose up to {} rows from the complete local catalog, aiming to fill that many whenever enough suitable tracks exist. Match named artists and genres strictly. Never substitute unrelated artists or genres for a specific request; if no tracks match, return []. Otherwise choose a varied, cohesive mix while also helping the listener discover music: include some less familiar tracks by related artists or genres when the request allows it, rather than selecting only their most-played songs. Treat likes, followed artists, and saved albums as strong positive signals; completion, replay, and seeking backward are positive listening signals; early skips are negative. A user's explicit mood or artist request takes priority over general taste signals. Avoid too many tracks by one artist. Return only a JSON array of row IDs. Catalog columns: id, title, artist, genre, familiarity (L=liked, P=played, N=new), behavior flags (l=liked, f=followed artist, a=saved album, h=heard percent, p=plays, c=completions, s=early skips, r=replays, b=backward seeks).\n{}", compact_metadata(theme, 120), mix_size, catalog)
    };
    let body = serde_json::json!({
        "contents": [{"parts": [{"text": prompt}]}],
        "generationConfig": {
            "responseMimeType": if include_answer { "application/json" } else { "text/plain" },
            "thinkingConfig": {"thinkingLevel": thinking.id()},
            "maxOutputTokens": if include_answer { 512 } else { 256 + (mix_size as u64 * 12) + if thinking == GeminiThinking::Low { 128 } else { 0 } }
        }
    });
    let response = google_ai_post(key, &body.to_string(), model)?;
    let value: serde_json::Value = serde_json::from_str(&response)
        .map_err(|_| "Google returned an unreadable response. Try again.".to_string())?;
    let usage_metadata = value
        .get("usageMetadata")
        .unwrap_or(&serde_json::Value::Null);
    let usage = AiUsage {
        prompt_tokens: usage_metadata
            .get("promptTokenCount")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        output_tokens: usage_metadata
            .get("candidatesTokenCount")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        thinking_tokens: usage_metadata
            .get("thoughtsTokenCount")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        total_tokens: usage_metadata
            .get("totalTokenCount")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
    };
    if let Some(message) = value.pointer("/error/message").and_then(|v| v.as_str()) {
        return Err(format!(
            "Google AI error: {}",
            compact_metadata(message, 240)
        ));
    }
    let candidates_response = value
        .get("candidates")
        .and_then(serde_json::Value::as_array);
    let Some(candidate) = candidates_response.and_then(|items| items.first()) else {
        let reason = value
            .pointer("/promptFeedback/blockReason")
            .and_then(serde_json::Value::as_str)
            .map(|reason| format!("Google blocked the prompt ({reason}). Try a different description."))
            .unwrap_or_else(|| "Google returned no candidate text. Try a shorter description or check your API key's model access.".to_string());
        return Err(reason);
    };
    let parts = candidate
        .pointer("/content/parts")
        .and_then(serde_json::Value::as_array);
    let text = parts
        .into_iter()
        .flatten()
        .filter(|part| part.get("thought").and_then(serde_json::Value::as_bool) != Some(true))
        .filter_map(|part| part.get("text").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    if text.trim().is_empty() {
        let reason = candidate
            .get("finishReason")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("no text part");
        return Err(format!("Google stopped before returning a mix ({reason}). Try again; if it repeats, check the developer output."));
    }
    let cleaned = text
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    let (ids, answer): (Vec<usize>, Option<String>) = if include_answer {
        let object_text = if cleaned.starts_with('{') && cleaned.ends_with('}') {
            cleaned
        } else if let (Some(start), Some(end)) = (cleaned.find('{'), cleaned.rfind('}')) {
            &cleaned[start..=end]
        } else {
            return Err("Google's response did not contain an answer. Try again.".into());
        };
        let object: serde_json::Value = serde_json::from_str(object_text)
            .map_err(|_| "Google's answer could not be read. Try again.".to_string())?;
        let answer = object
            .get("answer")
            .and_then(serde_json::Value::as_str)
            .map(|text| compact_metadata(text.trim(), 900))
            .filter(|text| !text.is_empty())
            .ok_or_else(|| "Google returned an empty answer. Try again.".to_string())?;
        let ids = object
            .get("ids")
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(serde_json::Value::as_u64)
                    .map(|id| id as usize)
                    .collect()
            })
            .unwrap_or_default();
        (ids, Some(answer))
    } else {
        let json_text = if cleaned.starts_with('[') && cleaned.ends_with(']') {
            cleaned
        } else if let (Some(start), Some(end)) = (cleaned.find('['), cleaned.rfind(']')) {
            &cleaned[start..=end]
        } else {
            return Err(
                "Google's response did not contain a song list. Try a simpler mood.".into(),
            );
        };
        let ids: Vec<usize> = serde_json::from_str(json_text).map_err(|_| {
            "Google's response was not a list of song ids. Try a simpler mood.".to_string()
        })?;
        (ids, None)
    };
    let mut seen = HashSet::new();
    let indices = ids
        .into_iter()
        .filter(|id| *id < candidates.len() && seen.insert(*id))
        .take(mix_size)
        .map(|id| candidates[id].index)
        .collect::<Vec<_>>();
    if indices.is_empty() && !include_answer {
        Err("No matching tracks were found in your local library. Try a broader artist, genre, or mood.".into())
    } else {
        Ok((indices, usage, answer))
    }
}

#[cfg(windows)]
fn google_ai_key_exists() -> bool {
    read_google_ai_key().ok().flatten().is_some()
}
#[cfg(not(windows))]
fn google_ai_key_exists() -> bool {
    false
}

#[cfg(windows)]
fn write_google_ai_key(value: &str) -> Result<(), String> {
    use windows_sys::Win32::Security::Credentials::{
        CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
    };
    let value = value.trim();
    if value.is_empty() {
        return Err("Paste a Google AI key first.".into());
    }
    if value.len() > 512 || !value.bytes().all(|byte| byte.is_ascii_graphic()) {
        return Err("That Google AI key contains invalid characters or is too long.".into());
    }
    let target = wide_null(GOOGLE_AI_KEY_TARGET);
    let username = wide_null("Google AI Studio");
    let mut credential = CREDENTIALW::default();
    credential.Type = CRED_TYPE_GENERIC;
    credential.TargetName = target.as_ptr() as *mut u16;
    credential.UserName = username.as_ptr() as *mut u16;
    credential.CredentialBlob = value.as_ptr() as *mut u8;
    credential.CredentialBlobSize = value.len() as u32;
    credential.Persist = CRED_PERSIST_LOCAL_MACHINE;
    unsafe {
        if CredWriteW(&credential, 0) == 0 {
            return Err("Windows could not securely save the Google AI key.".into());
        }
    }
    Ok(())
}
#[cfg(not(windows))]
fn write_google_ai_key(_: &str) -> Result<(), String> {
    Err("Google AI key storage is supported on Windows only.".into())
}

#[cfg(windows)]
fn read_google_ai_key() -> Result<Option<String>, String> {
    use windows_sys::Win32::Security::Credentials::{
        CredFree, CredReadW, CREDENTIALW, CRED_TYPE_GENERIC,
    };
    let target = wide_null(GOOGLE_AI_KEY_TARGET);
    let mut pointer: *mut CREDENTIALW = std::ptr::null_mut();
    unsafe {
        if CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut pointer) == 0 {
            return Ok(None);
        }
        let credential = &*pointer;
        let bytes = std::slice::from_raw_parts(
            credential.CredentialBlob,
            credential.CredentialBlobSize as usize,
        );
        let result = String::from_utf8(bytes.to_vec()).map_err(|_| {
            "Saved Google AI key is unreadable; remove and save it again.".to_string()
        });
        CredFree(pointer.cast());
        result.map(Some)
    }
}
#[cfg(not(windows))]
fn read_google_ai_key() -> Result<Option<String>, String> {
    Ok(None)
}

#[cfg(windows)]
fn delete_google_ai_key() -> Result<(), String> {
    use windows_sys::Win32::Security::Credentials::{CredDeleteW, CRED_TYPE_GENERIC};
    let target = wide_null(GOOGLE_AI_KEY_TARGET);
    unsafe {
        CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0);
    }
    Ok(())
}
#[cfg(not(windows))]
fn delete_google_ai_key() -> Result<(), String> {
    Err("Google AI key storage is supported on Windows only.".into())
}

#[cfg(windows)]
fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn version_parts(version: &str) -> Vec<u64> {
    version
        .trim_start_matches(['v', 'V'])
        .split('.')
        .map(|part| {
            part.chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse()
                .unwrap_or(0)
        })
        .collect()
}

fn is_newer_version(candidate: &str, current: &str) -> bool {
    let candidate = version_parts(candidate);
    let current = version_parts(current);
    let width = candidate.len().max(current.len());
    (0..width)
        .map(|index| candidate.get(index).copied().unwrap_or(0))
        .cmp((0..width).map(|index| current.get(index).copied().unwrap_or(0)))
        .is_gt()
}

#[cfg(windows)]
fn check_for_update() -> Result<Option<UpdateInfo>, String> {
    let response = winhttp_get(
        "api.github.com",
        "/repos/SupremyTommy/Lilt/releases/latest",
        "application/vnd.github+json",
        1024 * 1024,
    )?;
    let release: serde_json::Value = serde_json::from_slice(&response)
        .map_err(|_| "GitHub returned an invalid release response.".to_string())?;
    let tag = release["tag_name"]
        .as_str()
        .ok_or_else(|| "GitHub release did not include a version tag.".to_string())?;
    let current = env!("CARGO_PKG_VERSION");
    if !is_newer_version(tag, current) {
        return Ok(None);
    }
    let asset_url = release["assets"]
        .as_array()
        .and_then(|assets| {
            assets.iter().find_map(|asset| {
                let name = asset["name"].as_str()?;
                let url = asset["browser_download_url"].as_str()?;
                (name.to_ascii_lowercase().ends_with("windows-x64.msi")
                    || (name.to_ascii_lowercase().ends_with(".msi")
                        && name.to_ascii_lowercase().contains("x64")))
                .then(|| url.to_string())
            })
        })
        .ok_or_else(|| {
            "A newer Lilt release exists, but its Windows MSI was not found.".to_string()
        })?;
    Ok(Some(UpdateInfo {
        version: tag.trim_start_matches(['v', 'V']).to_string(),
        download_url: asset_url,
    }))
}

#[cfg(not(windows))]
fn check_for_update() -> Result<Option<UpdateInfo>, String> {
    Ok(None)
}

#[cfg(windows)]
fn open_default_apps_settings() -> Result<(), String> {
    std::process::Command::new("explorer.exe")
        .arg("ms-settings:defaultapps?registeredAppUser=Lilt")
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Could not open Windows Default Apps settings: {error}"))
}

#[cfg(not(windows))]
fn open_default_apps_settings() -> Result<(), String> {
    Err("Windows Default Apps settings are available only on Windows.".into())
}

#[cfg(windows)]
fn download_and_open_update(update: &UpdateInfo) -> Result<(), String> {
    let url = update
        .download_url
        .strip_prefix("https://")
        .ok_or_else(|| "The release download URL is not secure.".to_string())?;
    let (host, path) = url
        .split_once('/')
        .ok_or_else(|| "The release download URL is invalid.".to_string())?;
    let bytes = winhttp_get(
        host,
        &format!("/{path}"),
        "application/octet-stream",
        512 * 1024 * 1024,
    )?;
    if bytes.len() < 8 || !bytes.starts_with(b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1") {
        return Err("GitHub did not return a valid MSI installer.".into());
    }
    let installer = std::env::temp_dir().join(format!("Lilt-v{}-windows-x64.msi", update.version));
    fs::write(&installer, bytes)
        .map_err(|error| format!("Could not save the downloaded installer: {error}"))?;
    std::process::Command::new("msiexec.exe")
        .arg("/i")
        .arg(&installer)
        .spawn()
        .map_err(|error| {
            format!("Downloaded the installer, but could not open Windows Installer: {error}")
        })?;
    Ok(())
}

#[cfg(not(windows))]
fn download_and_open_update(_: &UpdateInfo) -> Result<(), String> {
    Err("The MSI updater is available only on Windows.".into())
}

#[cfg(windows)]
fn winhttp_get(host: &str, path: &str, accept: &str, max_bytes: usize) -> Result<Vec<u8>, String> {
    use std::{ffi::c_void, ptr};
    use windows_sys::Win32::Networking::WinHttp::*;
    unsafe {
        let agent = wide_null(LILT_USER_AGENT);
        let session = WinHttpOpen(
            agent.as_ptr(),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            ptr::null(),
            ptr::null(),
            0,
        );
        if session.is_null() {
            return Err("Could not start the secure internet connection.".into());
        }
        WinHttpSetTimeouts(session, 10_000, 10_000, 20_000, 60_000);
        let host_w = wide_null(host);
        let connection = WinHttpConnect(session, host_w.as_ptr(), 443, 0);
        if connection.is_null() {
            WinHttpCloseHandle(session);
            return Err(format!("Could not connect to {host}."));
        }
        let path_w = wide_null(path);
        let get = wide_null("GET");
        let request = WinHttpOpenRequest(
            connection,
            get.as_ptr(),
            path_w.as_ptr(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
            WINHTTP_FLAG_SECURE,
        );
        if request.is_null() {
            WinHttpCloseHandle(connection);
            WinHttpCloseHandle(session);
            return Err(format!("Could not prepare a request to {host}."));
        }
        let headers = wide_null(&format!(
            "Accept: {accept}\r\nUser-Agent: {LILT_USER_AGENT}\r\nX-GitHub-Api-Version: 2022-11-28\r\n"
        ));
        let ok = WinHttpSendRequest(
            request,
            headers.as_ptr(),
            (headers.len() - 1) as u32,
            ptr::null(),
            0,
            0,
            0,
        ) != 0
            && WinHttpReceiveResponse(request, ptr::null_mut()) != 0;
        let mut result = Ok(Vec::new());
        if !ok {
            result = Err(format!(
                "The request to {host} failed; check your internet connection."
            ));
        } else {
            let mut status = 0u32;
            let mut status_size = std::mem::size_of::<u32>() as u32;
            WinHttpQueryHeaders(
                request,
                WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                ptr::null(),
                (&mut status as *mut u32).cast(),
                &mut status_size,
                ptr::null_mut(),
            );
            if status != 200 {
                result = Err(format!("{host} returned HTTP {status}."));
            } else {
                let mut body = Vec::new();
                loop {
                    let mut available = 0;
                    if WinHttpQueryDataAvailable(request, &mut available) == 0 {
                        result = Err("The update download was interrupted.".into());
                        break;
                    }
                    if available == 0 {
                        break;
                    }
                    let start = body.len();
                    let take = (available as usize).min(64 * 1024);
                    if start.saturating_add(take) > max_bytes {
                        result = Err("The update download is larger than expected.".into());
                        break;
                    }
                    body.resize(start + take, 0);
                    let mut read = 0u32;
                    if WinHttpReadData(
                        request,
                        body[start..].as_mut_ptr().cast::<c_void>(),
                        take as u32,
                        &mut read,
                    ) == 0
                    {
                        result = Err("The update download was interrupted.".into());
                        break;
                    }
                    body.truncate(start + read as usize);
                }
                if result.is_ok() {
                    result = Ok(body);
                }
            }
        }
        WinHttpCloseHandle(request);
        WinHttpCloseHandle(connection);
        WinHttpCloseHandle(session);
        result
    }
}

#[cfg(windows)]
fn google_ai_post(key: &str, body: &str, model: GeminiModel) -> Result<String, String> {
    use std::{ffi::c_void, ptr};
    use windows_sys::Win32::Networking::WinHttp::*;
    unsafe {
        let agent = wide_null(LILT_USER_AGENT);
        let session = WinHttpOpen(
            agent.as_ptr(),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            ptr::null(),
            ptr::null(),
            0,
        );
        if session.is_null() {
            return Err("Could not start the secure Google connection.".into());
        }
        WinHttpSetTimeouts(session, 10_000, 10_000, 20_000, 30_000);
        let host = wide_null("generativelanguage.googleapis.com");
        let connection = WinHttpConnect(session, host.as_ptr(), 443, 0);
        if connection.is_null() {
            WinHttpCloseHandle(session);
            return Err("Could not connect to Google AI.".into());
        }
        let path = wide_null(&format!("/v1beta/models/{}:generateContent", model.id()));
        let post = wide_null("POST");
        let request = WinHttpOpenRequest(
            connection,
            post.as_ptr(),
            path.as_ptr(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
            WINHTTP_FLAG_SECURE,
        );
        if request.is_null() {
            WinHttpCloseHandle(connection);
            WinHttpCloseHandle(session);
            return Err("Could not prepare the Google AI request.".into());
        }
        let headers = wide_null(&format!(
            "Content-Type: application/json\r\nx-goog-api-key: {key}\r\n"
        ));
        let body_bytes = body.as_bytes();
        let sent = WinHttpSendRequest(
            request,
            headers.as_ptr(),
            (headers.len() - 1) as u32,
            body_bytes.as_ptr().cast(),
            body_bytes.len() as u32,
            body_bytes.len() as u32,
            0,
        ) != 0
            && WinHttpReceiveResponse(request, ptr::null_mut()) != 0;
        if !sent {
            WinHttpCloseHandle(request);
            WinHttpCloseHandle(connection);
            WinHttpCloseHandle(session);
            return Err(
                "Google AI request failed. Check your internet connection and API key.".into(),
            );
        }
        let mut status: u32 = 0;
        let mut status_size = std::mem::size_of::<u32>() as u32;
        WinHttpQueryHeaders(
            request,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            ptr::null(),
            (&mut status as *mut u32).cast(),
            &mut status_size,
            ptr::null_mut(),
        );
        let mut response = Vec::new();
        loop {
            let mut available = 0;
            if WinHttpQueryDataAvailable(request, &mut available) == 0 || available == 0 {
                break;
            }
            let take = available.min(64 * 1024);
            let start = response.len();
            response.resize(start + take as usize, 0);
            let mut read = 0;
            if WinHttpReadData(
                request,
                response[start..].as_mut_ptr().cast::<c_void>(),
                take,
                &mut read,
            ) == 0
            {
                response.truncate(start);
                break;
            }
            response.truncate(start + read as usize);
            if response.len() > 256 * 1024 {
                break;
            }
        }
        WinHttpCloseHandle(request);
        WinHttpCloseHandle(connection);
        WinHttpCloseHandle(session);
        let text = String::from_utf8_lossy(&response).into_owned();
        if status >= 400 {
            let message = serde_json::from_str::<serde_json::Value>(&text)
                .ok()
                .and_then(|v| {
                    v.pointer("/error/message")
                        .and_then(|m| m.as_str().map(str::to_string))
                })
                .unwrap_or_else(|| format!("Google AI returned HTTP {status}."));
            return Err(compact_metadata(&message, 220));
        }
        Ok(text)
    }
}
#[cfg(not(windows))]
fn google_ai_post(_: &str, _: &str, _: GeminiModel) -> Result<String, String> {
    Err("Google AI mix generation is supported on Windows only.".into())
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
    Ok(project.config_dir().join("settings.json"))
}

fn safe_export_name(name: &str) -> String {
    let safe = name
        .chars()
        .map(|character| {
            if character.is_control()
                || ['<', '>', ';', ':', '\"', '/', '\\', '|', '?', '*'].contains(&character)
            {
                '_'
            } else {
                character
            }
        })
        .collect::<String>();
    let safe = safe.trim().trim_matches('.').trim();
    if safe.is_empty() {
        "Lilt Playlist".into()
    } else {
        safe.into()
    }
}

fn unique_export_file_name(path: &Path, used: &mut HashSet<String>) -> String {
    let original = path
        .file_name()
        .and_then(|name| name.to_str())
        .map(safe_export_name)
        .unwrap_or_else(|| "audio-file".into());
    let file = Path::new(&original);
    let stem = file
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("audio-file");
    let extension = file.extension().and_then(|extension| extension.to_str());
    let build_name = |suffix: usize| {
        let candidate = if suffix == 1 {
            stem.to_owned()
        } else {
            format!("{stem} ({suffix})")
        };
        match extension {
            Some(extension) => format!("{candidate}.{extension}"),
            None => candidate,
        }
    };
    let mut suffix = 1;
    loop {
        let candidate = build_name(suffix);
        if used.insert(candidate.to_lowercase()) {
            return candidate;
        }
        suffix += 1;
    }
}

fn stage_playlist_export(
    playlist_name: &str,
    tracks: &[PathBuf],
    destination: &Path,
) -> Result<usize, String> {
    fs::create_dir_all(destination)
        .map_err(|error| format!("Could not create the export folder: {error}"))?;
    let mut used = HashSet::new();
    let mut entries = Vec::new();
    for path in tracks {
        if !path.is_file() {
            continue;
        }
        let file_name = unique_export_file_name(path, &mut used);
        fs::copy(path, destination.join(&file_name))
            .map_err(|error| format!("Could not copy {}: {error}", path.display()))?;
        entries.push(file_name);
    }
    let mut manifest = String::from("#EXTM3U\n");
    for entry in &entries {
        manifest.push_str(entry);
        manifest.push('\n');
    }
    let manifest_path = destination.join(format!("{}.m3u8", safe_export_name(playlist_name)));
    fs::write(&manifest_path, manifest)
        .map_err(|error| format!("Could not write the M3U8 playlist: {error}"))?;
    Ok(entries.len())
}

fn export_playlist_folder(
    playlist_name: &str,
    tracks: &[PathBuf],
    parent: &Path,
) -> Result<String, String> {
    let base = safe_export_name(playlist_name);
    let mut destination = parent.join(&base);
    let mut suffix = 2;
    while destination.exists() {
        destination = parent.join(format!("{base} ({suffix})"));
        suffix += 1;
    }
    let copied = stage_playlist_export(playlist_name, tracks, &destination)?;
    Ok(format!(
        "Exported {copied} song(s) and an M3U8 playlist to {}.",
        destination.display()
    ))
}

fn write_m3u8_file(tracks: &[PathBuf], destination: &Path) -> Result<String, String> {
    let mut contents = String::from("#EXTM3U\n");
    for path in tracks {
        if path.is_file() {
            contents.push_str(&path.to_string_lossy());
            contents.push('\n');
        }
    }
    fs::write(destination, contents).map_err(|error| error.to_string())?;
    Ok(format!(
        "Exported the M3U8 playlist to {}.",
        destination.display()
    ))
}

fn export_playlist_zip(
    playlist_name: &str,
    tracks: &[PathBuf],
    destination: &Path,
) -> Result<String, String> {
    let mut zip_path = destination.to_path_buf();
    if zip_path
        .extension()
        .is_none_or(|extension| !extension.eq_ignore_ascii_case("zip"))
    {
        zip_path.set_extension("zip");
    }
    let temp_root = std::env::temp_dir().join(format!(
        "lilt-playlist-export-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let staging = temp_root.join(safe_export_name(playlist_name));
    let result = (|| {
        let copied = stage_playlist_export(playlist_name, tracks, &staging)?;
        let output = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Compress-Archive -Path (Join-Path $env:LILT_EXPORT_SOURCE '*') -DestinationPath $env:LILT_EXPORT_ZIP -CompressionLevel Optimal -Force",
            ])
            .env("LILT_EXPORT_SOURCE", &staging)
            .env("LILT_EXPORT_ZIP", &zip_path)
            .output()
            .map_err(|error| format!("Could not start the Windows ZIP utility: {error}"))?;
        if !output.status.success() {
            let error = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            return Err(if error.is_empty() {
                "Windows could not create the ZIP archive.".into()
            } else {
                error
            });
        }
        Ok(format!(
            "Exported {copied} song(s) and an M3U8 playlist to {}.",
            zip_path.display()
        ))
    })();
    let _ = fs::remove_dir_all(&temp_root);
    result
}

fn fallback_preferences_path() -> Result<PathBuf, String> {
    if let Ok(executable) = std::env::current_exe() {
        if let Some(directory) = executable.parent() {
            return Ok(directory
                .join("LiltPlayerData")
                .join("config")
                .join("settings.json"));
        }
    }
    Ok(std::env::temp_dir()
        .join("LiltPlayerData")
        .join("config")
        .join("settings.json"))
}

fn read_preferences_file(path: &Path) -> Result<Preferences, String> {
    let bytes = fs::read(path)
        .map_err(|error| format!("Could not read settings at {}: {error}", path.display()))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("Could not parse settings at {}: {error}", path.display()))
}

fn load_preferences() -> Result<Preferences, String> {
    let mut candidates = [preferences_path()?, fallback_preferences_path()?]
        .into_iter()
        .filter(|path| path.exists())
        .collect::<Vec<_>>();
    candidates.sort_by_key(|path| {
        fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .unwrap_or(UNIX_EPOCH)
    });
    if candidates.is_empty() {
        return Ok(Preferences::default());
    }
    let mut last_error = None;
    for path in candidates.into_iter().rev() {
        match read_preferences_file(&path) {
            Ok(preferences) => return Ok(preferences),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| "Could not read the settings file.".into()))
}

fn save_preferences(preferences: &Preferences) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(&preferences)
        .map_err(|error| format!("Could not encode settings: {error}"))?;
    let primary = preferences_path()?;
    match write_settings_atomically(&primary, &bytes) {
        Ok(()) => Ok(()),
        Err(primary_error) => {
            let fallback = fallback_preferences_path()?;
            if fallback == primary {
                return Err(format!(
                    "Could not save settings at {}: {primary_error}",
                    primary.display()
                ));
            }
            write_settings_atomically(&fallback, &bytes).map_err(|fallback_error| {
                format!(
                    "Could not save settings at {} ({primary_error}) or fallback {} ({fallback_error}).",
                    primary.display(),
                    fallback.display()
                )
            })
        }
    }
}

fn write_settings_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "settings path has no parent",
        )
    })?;
    fs::create_dir_all(parent)?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = parent.join(format!(".settings-{}-{nonce}.tmp", std::process::id()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    match fs::rename(&temporary, path) {
        Ok(()) => Ok(()),
        Err(rename_error) => {
            let _ = fs::remove_file(&temporary);
            Err(rename_error)
        }
    }
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
    ui: &mut egui::Ui,
    cover: Option<&TextureHandle>,
    rect: egui::Rect,
    seed: usize,
) {
    if let Some(cover) = cover {
        ui.put(
            rect,
            egui::Image::new((cover.id(), rect.size())).corner_radius(egui::CornerRadius::same(14)),
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
            egui::CornerRadius::same(14),
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
) -> egui::Response {
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
        .rect_filled(rect, egui::CornerRadius::same(18), fill);
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
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
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
    let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    let mut startup_folder = None;
    let mut startup_file = None;
    let mut argument_index = 0;
    while argument_index < arguments.len() {
        if arguments[argument_index] == "--folder" {
            if let Some(folder) = arguments.get(argument_index + 1) {
                startup_folder = Some(PathBuf::from(folder));
                argument_index += 1;
            }
        } else if startup_file.is_none() {
            let path = PathBuf::from(&arguments[argument_index]);
            if path.is_file() {
                startup_file = Some(path);
            }
        }
        argument_index += 1;
    }
    let app_icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/app_icon.png"))
        .unwrap_or_default();
    let start_maximized = load_preferences()
        .map(|preferences| preferences.start_maximized)
        .unwrap_or(true);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(APP_NAME)
            .with_icon(app_icon)
            .with_decorations(false)
            .with_maximized(start_maximized)
            .with_inner_size([1680.0, 1040.0])
            .with_min_inner_size([1120.0, 720.0]),
        ..Default::default()
    };
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(move |_creation_context| {
            Ok(Box::new(PlayerApp::new_with_folder(
                startup_folder,
                startup_file,
            )))
        }),
    )
}


// Persistent library index and lazy embedded-artwork helpers.
//
// The index stores tag metadata (never image bytes) so a warm start can render
// the library before the incremental verification scan finishes. Unchanged
// files are matched by size + modification time and reused without re-reading
// their tags.

const LIBRARY_CACHE_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct LibraryIndex {
    version: u32,
    #[serde(default)]
    folders: Vec<PathBuf>,
    #[serde(default)]
    tracks: Vec<Track>,
}

fn timing_enabled() -> bool {
    std::env::var_os("LILT_TIMING").is_some()
}

fn library_cache_path() -> Result<PathBuf, String> {
    let project = ProjectDirs::from("com", "Lilt", "Lilt Player")
        .ok_or_else(|| "Could not locate the local library cache directory.".to_string())?;
    Ok(project
        .data_local_dir()
        .join("library")
        .join("index.json"))
}

fn library_search_text(track: &Track) -> String {
    format!(
        "{} {} {} {}",
        track.title, track.artist, track.album, track.genre
    )
    .to_lowercase()
}

fn decode_library_cache(bytes: &[u8]) -> Option<Vec<Track>> {
    let index: LibraryIndex = serde_json::from_slice(bytes).ok()?;
    if index.version != LIBRARY_CACHE_VERSION || index.tracks.is_empty() {
        return None;
    }
    let mut tracks = index.tracks;
    for track in &mut tracks {
        track.artwork = None;
        if track.search_text.is_empty() {
            track.search_text = library_search_text(track);
        }
    }
    Some(tracks)
}

fn encode_library_cache(folders: &[PathBuf], tracks: &[Track]) -> Result<Vec<u8>, String> {
    serde_json::to_vec(&LibraryIndex {
        version: LIBRARY_CACHE_VERSION,
        folders: folders.to_vec(),
        tracks: tracks.to_vec(),
    })
    .map_err(|error| format!("Could not encode the library cache: {error}"))
}

fn load_library_cache() -> Option<Vec<Track>> {
    let started = Instant::now();
    let path = library_cache_path().ok()?;
    let bytes = fs::read(path).ok()?;
    let tracks = decode_library_cache(&bytes)?;
    if timing_enabled() {
        eprintln!(
            "[lilt] library cache loaded: {} tracks in {:?}",
            tracks.len(),
            started.elapsed()
        );
    }
    Some(tracks)
}

fn save_library_cache(folders: &[PathBuf], tracks: &[Track]) -> Result<(), String> {
    let started = Instant::now();
    let bytes = encode_library_cache(folders, tracks)?;
    let path = library_cache_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create the library cache folder: {error}"))?;
    }
    write_settings_atomically(&path, &bytes)
        .map_err(|error| format!("Could not save the library cache: {error}"))?;
    if timing_enabled() {
        eprintln!(
            "[lilt] library cache saved: {} tracks in {:?}",
            tracks.len(),
            started.elapsed()
        );
    }
    Ok(())
}

fn cached_track_map() -> Option<Arc<HashMap<PathBuf, Track>>> {
    load_library_cache().map(|tracks| {
        Arc::new(
            tracks
                .into_iter()
                .map(|track| (track.path.clone(), track))
                .collect(),
        )
    })
}

/// Read the first embedded cover only when a visible row actually needs it.
fn read_embedded_artwork(path: &Path) -> Option<Arc<Vec<u8>>> {
    let tagged = Probe::open(path)
        .and_then(|probe| probe.read())
        .ok()?;
    let tag = tagged.primary_tag()?;
    let data = tag.pictures().first()?.data();
    if data.is_empty() || data.len() > MAX_CACHED_ARTWORK_BYTES {
        return None;
    }
    Some(Arc::new(data.to_vec()))
}

/// Stable in-memory signature of the inputs that affect duplicate suggestions.
/// Recomputing this is cheap compared to hashing audio files again.
fn duplicate_scan_signature(tracks: &[Track], liked_songs: &HashSet<PathBuf>) -> u64 {
    let mut hasher = DefaultHasher::new();
    tracks.len().hash(&mut hasher);
    for track in tracks {
        track.path.hash(&mut hasher);
        track.size_bytes.hash(&mut hasher);
        track.modified_ms.hash(&mut hasher);
        liked_songs.contains(&track.path).hash(&mut hasher);
    }
    hasher.finish()
}

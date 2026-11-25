// Windows integration for the background player:
// - a single-instance mutex so relaunching Lilt shows the existing window
// - a small launch-request file for Quick Play handoff between instances
// - HKCU registration for "Quick Play with Lilt" in Explorer
// - HKCU Run registration for starting Lilt in the background at login

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct LaunchRequest {
    kind: String,
    #[serde(default)]
    path: Option<std::path::PathBuf>,
    #[serde(default)]
    nonce: u64,
}

impl LaunchRequest {
    fn show() -> Self {
        Self {
            kind: "show".into(),
            path: None,
            nonce: request_nonce(),
        }
    }

    fn quick_play(path: std::path::PathBuf) -> Self {
        Self {
            kind: "quick_play".into(),
            path: Some(path),
            nonce: request_nonce(),
        }
    }
}

fn request_nonce() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos() as u64)
        ^ (std::process::id() as u64).wrapping_mul(1_000_000_007)
}

fn launch_request_path() -> std::path::PathBuf {
    std::env::temp_dir().join("lilt-player-launch-request.json")
}

fn clear_stale_launch_request() {
    let _ = std::fs::remove_file(launch_request_path());
}

fn send_launch_request(request: &LaunchRequest) -> Result<(), String> {
    let path = launch_request_path();
    let bytes = serde_json::to_vec(request)
        .map_err(|error| format!("Could not encode the Lilt launch request: {error}"))?;
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&temporary, bytes)
        .map_err(|error| format!("Could not write the Lilt launch request: {error}"))?;
    std::fs::rename(&temporary, &path)
        .map_err(|error| format!("Could not deliver the Lilt launch request: {error}"))
}

/// Watches for requests from a second Lilt process. The thread stops when the
/// app drops its receiver.
fn start_launch_request_watcher(
    context: egui::Context,
) -> std::sync::mpsc::Receiver<LaunchRequest> {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let path = launch_request_path();
        loop {
            if let Ok(bytes) = std::fs::read(&path) {
                // Remove before forwarding so the same request is never handled
                // twice, and so a malformed file cannot cause a polling loop.
                let _ = std::fs::remove_file(&path);
                if let Ok(request) = serde_json::from_slice::<LaunchRequest>(&bytes) {
                    if sender.send(request).is_err() {
                        return;
                    }
                    context.request_repaint();
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(650));
        }
    });
    receiver
}

#[cfg(target_os = "windows")]
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(target_os = "windows")]
fn acquire_single_instance() -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS};
    use windows_sys::Win32::System::Threading::CreateMutexW;

    let name = wide("Local\\LiltPlayer.SingleInstance.v1");
    let handle = unsafe { CreateMutexW(std::ptr::null(), 1, name.as_ptr()) };
    if handle.is_null() {
        // If the mutex cannot be created, allow the app to start rather than
        // blocking the user entirely.
        return true;
    }
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        unsafe {
            CloseHandle(handle);
        }
        false
    } else {
        // The raw handle intentionally stays open for the life of the process.
        true
    }
}

#[cfg(not(target_os = "windows"))]
fn acquire_single_instance() -> bool {
    true
}

#[cfg(target_os = "windows")]
fn registry_create_key(path: &str) -> Result<windows_sys::Win32::System::Registry::HKEY, String> {
    use windows_sys::Win32::System::Registry::{RegCloseKey, RegCreateKeyW, HKEY, HKEY_CURRENT_USER};

    let mut current: HKEY = HKEY_CURRENT_USER;
    for part in path.split('\\').filter(|part| !part.is_empty()) {
        let name = wide(part);
        let mut next: HKEY = std::ptr::null_mut();
        let status = unsafe { RegCreateKeyW(current, name.as_ptr(), &mut next) };
        if current != HKEY_CURRENT_USER {
            unsafe {
                RegCloseKey(current);
            }
        }
        if status != 0 {
            return Err(format!(
                "Windows could not open the registry key HKCU\\{path} (error {status})."
            ));
        }
        current = next;
    }
    Ok(current)
}

#[cfg(target_os = "windows")]
fn registry_set_string(path: &str, name: Option<&str>, value: &str) -> Result<(), String> {
    use windows_sys::Win32::System::Registry::{RegCloseKey, RegSetValueExW, REG_SZ};

    let key = registry_create_key(path)?;
    let name = name.map(wide);
    let value = wide(value);
    let status = unsafe {
        RegSetValueExW(
            key,
            name.as_ref().map_or(std::ptr::null(), |name| name.as_ptr()),
            0,
            REG_SZ,
            value.as_ptr() as *const u8,
            (value.len() * std::mem::size_of::<u16>()) as u32,
        )
    };
    unsafe {
        RegCloseKey(key);
    }
    if status != 0 {
        return Err(format!(
            "Windows could not write HKCU\\{path} (error {status})."
        ));
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn registry_delete_tree(path: &str) {
    use windows_sys::Win32::System::Registry::{RegDeleteTreeW, HKEY_CURRENT_USER};

    let name = wide(path);
    unsafe {
        RegDeleteTreeW(HKEY_CURRENT_USER, name.as_ptr());
    }
}

#[cfg(target_os = "windows")]
fn registry_delete_value(path: &str, name: &str) {
    use windows_sys::Win32::System::Registry::{RegCloseKey, RegDeleteValueW};

    let Ok(key) = registry_create_key(path) else {
        return;
    };
    let name = wide(name);
    unsafe {
        RegDeleteValueW(key, name.as_ptr());
        RegCloseKey(key);
    }
}

fn quick_play_command(placeholder: &str) -> Result<String, String> {
    let executable = std::env::current_exe()
        .map_err(|error| format!("Could not locate the Lilt executable: {error}"))?;
    Ok(format!(
        "\"{}\" --quick-play \"{placeholder}\"",
        executable.display()
    ))
}

#[cfg(target_os = "windows")]
fn set_explorer_quick_play(enabled: bool) -> Result<(), String> {
    let commands = [
        (
            "Software\\Classes\\SystemFileAssociations\\audio\\shell\\LiltQuickPlay",
            "Quick Play with Lilt",
            false,
        ),
        (
            "Software\\Classes\\Directory\\shell\\LiltQuickPlay",
            "Quick Play folder with Lilt",
            false,
        ),
        (
            "Software\\Classes\\Directory\\Background\\shell\\LiltQuickPlay",
            "Quick Play this folder with Lilt",
            true,
        ),
    ];
    if !enabled {
        for (key, _, _) in commands {
            registry_delete_tree(key);
        }
        return Ok(());
    }
    let icon = std::env::current_exe()
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    for (key, label, is_background) in commands {
        registry_set_string(key, None, label)?;
        if !icon.is_empty() {
            registry_set_string(key, Some("Icon"), &icon)?;
        }
        // Folder-background commands receive the current directory through %V;
        // file and folder selections use %1.
        let placeholder = if is_background { "%V" } else { "%1" };
        registry_set_string(
            &format!("{key}\\command"),
            None,
            &quick_play_command(placeholder)?,
        )?;
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn set_explorer_quick_play(_enabled: bool) -> Result<(), String> {
    Ok(())
}

#[cfg(target_os = "windows")]
fn set_start_with_windows(enabled: bool) -> Result<(), String> {
    const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
    if !enabled {
        registry_delete_value(RUN_KEY, "Lilt");
        return Ok(());
    }
    let executable = std::env::current_exe()
        .map_err(|error| format!("Could not locate the Lilt executable: {error}"))?;
    registry_set_string(
        RUN_KEY,
        Some("Lilt"),
        &format!("\"{}\" --background", executable.display()),
    )
}

#[cfg(not(target_os = "windows"))]
fn set_start_with_windows(_enabled: bool) -> Result<(), String> {
    Ok(())
}

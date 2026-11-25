// Watches Windows audio sessions so Lilt can duck its own background music when
// another app starts playing. The check is intentionally conservative: a
// session counts as audio only while another process has an active session with
// a non-zero peak, so paused browser tabs and notification sounds are ignored.

#[cfg(all(target_os = "windows", not(test)))]
fn other_audio_is_playing(own_pid: u32) -> bool {
    use windows::core::Interface;
    use windows::Win32::Media::Audio::Endpoints::IAudioMeterInformation;
    use windows::Win32::Media::Audio::{
        eConsole, eRender, AudioSessionStateActive, IAudioSessionControl2,
        IAudioSessionManager2, IMMDeviceEnumerator, MMDeviceEnumerator,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
    };

    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let detected = (|| -> windows::core::Result<bool> {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?;
            let manager: IAudioSessionManager2 = device.Activate(CLSCTX_ALL, None)?;
            let sessions = manager.GetSessionEnumerator()?;
            for index in 0..sessions.GetCount()? {
                let Ok(control) = sessions.GetSession(index) else {
                    continue;
                };
                let Ok(control) = control.cast::<IAudioSessionControl2>() else {
                    continue;
                };
                // S_OK means the session belongs to system sounds. S_FALSE is a
                // success HRESULT too, so compare the code instead of using ok().
                if control.IsSystemSoundsSession().0 == 0 {
                    continue;
                }
                let process_id = control.GetProcessId().unwrap_or_default();
                if process_id == 0 || process_id == own_pid {
                    continue;
                }
                if control.GetState().ok() != Some(AudioSessionStateActive) {
                    continue;
                }
                // A session stays active while a player is open even if it is
                // silent. Use its peak meter when available so silence does not
                // duck Lilt forever.
                if let Ok(meter) = control.cast::<IAudioMeterInformation>() {
                    if meter.GetPeakValue().unwrap_or(0.0) > 0.0002 {
                        return Ok(true);
                    }
                } else {
                    return Ok(true);
                }
            }
            Ok(false)
        })();
        CoUninitialize();
        detected.unwrap_or(false)
    }
}

#[cfg(all(not(target_os = "windows"), not(test)))]
fn other_audio_is_playing(_own_pid: u32) -> bool {
    false
}

/// Starts a lightweight polling thread. It only sends when activity changes,
/// and exits automatically once the receiver is dropped.
#[cfg(not(test))]
fn start_audio_activity_monitor() -> std::sync::mpsc::Receiver<bool> {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let own_pid = std::process::id();
        let mut last = false;
        loop {
            let playing = other_audio_is_playing(own_pid);
            if playing != last {
                last = playing;
                if sender.send(playing).is_err() {
                    return;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(700));
        }
    });
    receiver
}

#[cfg(test)]
fn start_audio_activity_monitor() -> std::sync::mpsc::Receiver<bool> {
    // Tests should not spawn COM polling threads for every app instance.
    std::sync::mpsc::channel().1
}

#[cfg(windows)]
pub mod windows_audio {
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{
        eMultimedia, eRender, IMMDeviceEnumerator, MMDeviceEnumerator,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED,
    };

    fn get_endpoint_volume() -> Result<IAudioEndpointVolume, String> {
        unsafe {
            // Ignore error if already initialized on this thread
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);

            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                    .map_err(|e| format!("CoCreateInstance MMDeviceEnumerator failed: {e}"))?;

            let device = enumerator
                .GetDefaultAudioEndpoint(eRender, eMultimedia)
                .map_err(|e| format!("GetDefaultAudioEndpoint failed: {e}"))?;

            let endpoint: IAudioEndpointVolume = device
                .Activate(CLSCTX_ALL, None)
                .map_err(|e| format!("Activate IAudioEndpointVolume failed: {e}"))?;

            Ok(endpoint)
        }
    }

    pub fn get_volume() -> Result<f32, String> {
        let endpoint = get_endpoint_volume()?;
        unsafe {
            endpoint
                .GetMasterVolumeLevelScalar()
                .map_err(|e| format!("GetMasterVolumeLevelScalar failed: {e}"))
        }
    }

    pub fn set_volume(level: f32) -> Result<f32, String> {
        let endpoint = get_endpoint_volume()?;
        let clamped = level.clamp(0.0, 1.0);
        unsafe {
            endpoint
                .SetMasterVolumeLevelScalar(clamped, std::ptr::null())
                .map_err(|e| format!("SetMasterVolumeLevelScalar failed: {e}"))?;

            // If setting volume > 0, make sure it's unmuted
            if clamped > 0.0 {
                let _ = endpoint.SetMute(false, std::ptr::null());
            } else {
                let _ = endpoint.SetMute(true, std::ptr::null());
            }

            Ok(clamped)
        }
    }

    pub fn is_muted() -> Result<bool, String> {
        let endpoint = get_endpoint_volume()?;
        unsafe {
            endpoint
                .GetMute()
                .map(|b| b.as_bool())
                .map_err(|e| format!("GetMute failed: {e}"))
        }
    }

    pub fn set_mute(mute: bool) -> Result<bool, String> {
        let endpoint = get_endpoint_volume()?;
        unsafe {
            endpoint
                .SetMute(mute, std::ptr::null())
                .map_err(|e| format!("SetMute failed: {e}"))?;
            Ok(mute)
        }
    }
}

#[cfg(windows)]
pub use windows_audio::*;

#[cfg(not(windows))]
pub fn get_volume() -> Result<f32, String> {
    Ok(1.0)
}

#[cfg(not(windows))]
pub fn set_volume(_level: f32) -> Result<f32, String> {
    Ok(1.0)
}

#[cfg(not(windows))]
pub fn is_muted() -> Result<bool, String> {
    Ok(false)
}

#[cfg(not(windows))]
pub fn set_mute(_mute: bool) -> Result<bool, String> {
    Ok(false)
}

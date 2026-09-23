/// Below this system output volume the cue is taken not to carry (R5a).
pub const VOLUME_FLOOR: f32 = 0.10;

/// What can be learned of the default output. `None` is unknown, which
/// passes: a probe that failed is not evidence the cue cannot be heard.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OutputState {
    pub present: bool,
    pub muted: Option<bool>,
    /// 0.0 to 1.0.
    pub volume: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OutputProblem {
    NoOutput,
    Muted,
    Quiet(f32),
}

impl std::fmt::Display for OutputProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OutputProblem::NoOutput => write!(f, "no sound output, so the alert cannot be heard"),
            OutputProblem::Muted => write!(f, "sound is muted, so the alert cannot be heard"),
            OutputProblem::Quiet(v) => write!(
                f,
                "the volume is at {:.0}%, too low for the alert to carry",
                v * 100.0
            ),
        }
    }
}

impl OutputState {
    /// Why the cue would not be heard, if it would not.
    pub fn problem(&self) -> Option<OutputProblem> {
        if !self.present {
            return Some(OutputProblem::NoOutput);
        }
        if self.muted == Some(true) {
            return Some(OutputProblem::Muted);
        }
        match self.volume {
            Some(v) if v < VOLUME_FLOOR => Some(OutputProblem::Quiet(v)),
            _ => None,
        }
    }
}

/// The default output's name, cheap enough to poll: a change is when the
/// full probe runs again (R5a).
pub fn default_output_name() -> Option<String> {
    use cpal::traits::{DeviceTrait, HostTrait};
    let device = cpal::default_host().default_output_device()?;
    device.description().ok().map(|d| d.name().to_string())
}

/// Probe the default output. Spawns `osascript` on macOS, so call it off any
/// path that must be quick.
pub fn output_state() -> OutputState {
    use cpal::traits::HostTrait;
    let present = cpal::default_host().default_output_device().is_some();
    if !present {
        return OutputState {
            present,
            muted: None,
            volume: None,
        };
    }
    let (muted, volume) = platform::probe();
    OutputState {
        present,
        muted,
        volume,
    }
}

#[cfg(target_os = "macos")]
mod platform {
    /// Handy's `output muted of (get volume settings)`, extended to read the
    /// volume in the same call. A device with no volume control (HDMI, some
    /// interfaces) answers `missing value`, which reads as unknown.
    pub fn probe() -> (Option<bool>, Option<f32>) {
        let script = "set s to (get volume settings)\n\
                      return ((output volume of s) as text) & \",\" & ((output muted of s) as text)";
        let Ok(out) = std::process::Command::new("osascript")
            .args(["-e", script])
            .output()
        else {
            return (None, None);
        };
        if !out.status.success() {
            return (None, None);
        }
        super::parse_volume_settings(String::from_utf8_lossy(&out.stdout).trim())
    }
}

#[cfg(windows)]
mod platform {
    /// Handy's `IAudioEndpointVolume::GetMute`, with the master level read
    /// from the same endpoint.
    pub fn probe() -> (Option<bool>, Option<f32>) {
        use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
        use windows::Win32::Media::Audio::{
            IMMDeviceEnumerator, MMDeviceEnumerator, eMultimedia, eRender,
        };
        use windows::Win32::System::Com::{
            CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
        };
        unsafe {
            // A no-op when COM is already initialized on this thread.
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let endpoint = (|| {
                let devices: IMMDeviceEnumerator =
                    CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
                let device = devices.GetDefaultAudioEndpoint(eRender, eMultimedia).ok()?;
                device
                    .Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None)
                    .ok()
            })();
            let Some(endpoint) = endpoint else {
                return (None, None);
            };
            let muted = endpoint.GetMute().ok().map(|m| m.as_bool());
            let volume = endpoint.GetMasterVolumeLevelScalar().ok();
            (muted, volume)
        }
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
mod platform {
    pub fn probe() -> (Option<bool>, Option<f32>) {
        (None, None)
    }
}

/// `"<volume 0-100>,<true|false>"`, either part possibly `missing value`.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_volume_settings(text: &str) -> (Option<bool>, Option<f32>) {
    let (volume, muted) = text.split_once(',').unwrap_or((text, ""));
    let volume = volume
        .trim()
        .parse::<f32>()
        .ok()
        .map(|v| (v / 100.0).clamp(0.0, 1.0));
    let muted = match muted.trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    };
    (muted, volume)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(present: bool, muted: Option<bool>, volume: Option<f32>) -> OutputState {
        OutputState {
            present,
            muted,
            volume,
        }
    }

    #[test]
    fn an_absent_muted_or_quiet_output_is_named() {
        assert_eq!(
            state(false, None, None).problem(),
            Some(OutputProblem::NoOutput)
        );
        assert_eq!(
            state(true, Some(true), Some(0.8)).problem(),
            Some(OutputProblem::Muted)
        );
        assert_eq!(
            state(true, Some(false), Some(0.05)).problem(),
            Some(OutputProblem::Quiet(0.05))
        );
    }

    #[test]
    fn an_audible_or_unknown_output_passes() {
        assert_eq!(state(true, Some(false), Some(0.5)).problem(), None);
        assert_eq!(state(true, Some(false), Some(VOLUME_FLOOR)).problem(), None);
        assert_eq!(state(true, None, None).problem(), None);
    }

    #[test]
    fn volume_settings_parse_with_missing_values() {
        assert_eq!(parse_volume_settings("50,false"), (Some(false), Some(0.5)));
        assert_eq!(parse_volume_settings("0,true"), (Some(true), Some(0.0)));
        assert_eq!(
            parse_volume_settings("missing value,missing value"),
            (None, None)
        );
    }
}

#[cfg(test)]
mod hardware {
    /// `cargo test -p sfx -- --ignored --nocapture` prints this machine's
    /// output as the probe sees it.
    #[test]
    #[ignore = "reads the real output device"]
    fn hardware_output_probe_reports_this_machine() {
        let state = super::output_state();
        println!("{state:?} -> {:?}", state.problem());
        assert!(state.present);
    }
}

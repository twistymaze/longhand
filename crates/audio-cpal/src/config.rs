//! Device resolution and stream configuration.
//!
//! `preferred_config` is Handy's `get_preferred_config()` on the cpal 0.18
//! API: the device's own default rate, never a forced one (Bluetooth codecs
//! and some drivers misbehave at a rate they did not choose), with the best
//! sample format that rate offers. See PROVENANCE.md.

use std::collections::HashMap;
use std::sync::Mutex;

use cpal::traits::{DeviceTrait, HostTrait};
use cpal::{Device, InterfaceType, SampleFormat, SupportedStreamConfig};

use audio::Error;

pub fn device_name(device: &Device) -> String {
    device.to_string()
}

/// The named input, or the default one. When the default is a Bluetooth
/// headset and a wired or built-in input exists, the wired one is used:
/// opening a headset's microphone forces the low-rate hands-free profile,
/// which gates the room to silence between words. Only the platform default
/// is replaced; a device the user names is respected. This follows anarlog's
/// `active_mic_device`, using the transport cpal reports (WASAPI does;
/// CoreAudio does not yet, so on macOS the default is kept).
pub fn resolve_input(
    host: &cpal::Host,
    name: Option<&str>,
    bluetooth: &Mutex<HashMap<String, bool>>,
) -> Result<Device, Error> {
    if let Some(name) = name {
        return host
            .input_devices()
            .map_err(|_| Error::MicOpenFailed)?
            .find(|d| device_name(d) == name)
            .ok_or(Error::NoInputDevice);
    }
    let default = host.default_input_device().ok_or(Error::NoInputDevice)?;
    // `description()` enumerates the device's configs, which costs more than
    // the whole open; the verdict is remembered per device so only
    // `prepare` pays it.
    let default_name = device_name(&default);
    let cached = bluetooth
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&default_name)
        .copied();
    let default_is_bluetooth = cached.unwrap_or_else(|| {
        let verdict = is_bluetooth(&default);
        bluetooth
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(default_name, verdict);
        verdict
    });
    if !default_is_bluetooth {
        return Ok(default);
    }
    let replacement = host
        .input_devices()
        .ok()
        .and_then(|mut devices| devices.find(|d| !is_bluetooth(d) && is_physical(d)));
    match replacement {
        Some(device) => {
            log::info!(
                "default input '{}' is Bluetooth; using '{}' instead",
                device_name(&default),
                device_name(&device)
            );
            Ok(device)
        }
        None => Ok(default),
    }
}

fn is_bluetooth(device: &Device) -> bool {
    device
        .description()
        .is_ok_and(|d| d.interface_type() == InterfaceType::Bluetooth)
}

fn is_physical(device: &Device) -> bool {
    device.description().is_ok_and(|d| {
        matches!(
            d.interface_type(),
            InterfaceType::BuiltIn
                | InterfaceType::Usb
                | InterfaceType::Pci
                | InterfaceType::FireWire
                | InterfaceType::Thunderbolt
                | InterfaceType::Line
        )
    })
}

fn format_score(format: SampleFormat) -> u8 {
    // Prioritize F32 > I16 > I32 > others
    match format {
        SampleFormat::F32 => 4,
        SampleFormat::I16 => 3,
        SampleFormat::I32 => 2,
        _ => 1,
    }
}

pub fn preferred_config(device: &Device) -> Result<SupportedStreamConfig, Error> {
    let default_config = device
        .default_input_config()
        .map_err(|e| Error::MicStreamInitializationFailed(e.to_string()))?;
    let target_rate = default_config.sample_rate();

    let supported = match device.supported_input_configs() {
        Ok(configs) => configs,
        Err(e) => {
            log::warn!("could not enumerate input configs ({e}); using the device default");
            return Ok(default_config);
        }
    };
    // The first range wins a tie, as in Handy.
    let best = supported
        .filter(|range| range.contains_rate(target_rate))
        .fold(
            None,
            |best: Option<cpal::SupportedStreamConfigRange>, range| match best {
                Some(b)
                    if format_score(range.sample_format()) <= format_score(b.sample_format()) =>
                {
                    Some(b)
                }
                _ => Some(range),
            },
        );
    match best {
        Some(range) => Ok(range.with_sample_rate(target_rate)),
        None => {
            // Exotic or virtual devices: no range contains their own default.
            log::warn!("no input config matched the default rate {target_rate}; using the default");
            Ok(default_config)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_beats_sixteen_bit_beats_thirty_two() {
        assert!(format_score(SampleFormat::F32) > format_score(SampleFormat::I16));
        assert!(format_score(SampleFormat::I16) > format_score(SampleFormat::I32));
        assert!(format_score(SampleFormat::I32) > format_score(SampleFormat::U8));
    }
}

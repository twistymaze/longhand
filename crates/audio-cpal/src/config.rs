//! Device resolution and stream configuration.
//!
//! `preferred_config` is Handy's `get_preferred_config()` on the cpal 0.18
//! API: the device's own default rate, never a forced one (Bluetooth codecs
//! and some drivers misbehave at a rate they did not choose), with the best
//! sample format that rate offers. See PROVENANCE.md.

use std::collections::HashMap;
use std::sync::Mutex;

use cpal::traits::{DeviceTrait, HostTrait};
use cpal::{Device, DeviceDescription, InterfaceType, SampleFormat, SupportedStreamConfig};

use audio::Error;

use crate::{Input, MicChoice};

/// The name of a device whose description cpal cannot read.
pub const UNKNOWN_DEVICE_NAME: &str = "Unknown microphone";

/// The device's name, or [`UNKNOWN_DEVICE_NAME`]. cpal 0.18's `Display` for
/// `Device` returns an error when the description cannot be read, and
/// `to_string()` then panics.
pub fn device_name(device: &Device) -> String {
    name_or_fallback(device.description())
}

/// Only `name()`: the `Display` of a description adds the manufacturer and
/// the device type.
fn name_or_fallback<E>(description: Result<DeviceDescription, E>) -> String {
    description
        .map(|d| d.name().to_owned())
        .unwrap_or_else(|_| UNKNOWN_DEVICE_NAME.to_owned())
}

/// cpal's stable device ID in its saved form (KTD1): the CoreAudio device UID
/// or the WASAPI endpoint ID, after the host name. Empty when the backend
/// cannot read it; an empty ID never matches a choice.
pub fn device_id(device: &Device) -> String {
    device.id().map(|id| id.to_string()).unwrap_or_default()
}

/// The key of a device in the per-device caches: its ID, so two identical
/// microphones do not share an entry (KTD1), or its name when the ID is
/// missing.
pub fn cache_key(device: &Device) -> String {
    let id = device_id(device);
    if id.is_empty() {
        device_name(device)
    } else {
        id
    }
}

pub fn input_of(device: &Device) -> Input {
    Input {
        id: device_id(device),
        name: device_name(device),
    }
}

/// The index of the device that `choice` names: the device with its ID
/// first, else the first device with its exact name (KTD1). Names are not
/// unique, so the name is only the second test.
pub fn find_choice(devices: &[Input], choice: &MicChoice) -> Option<usize> {
    let by_id = (!choice.id.is_empty())
        .then(|| devices.iter().position(|d| d.id == choice.id))
        .flatten();
    by_id.or_else(|| devices.iter().position(|d| d.name == choice.label))
}

/// The chosen input, or the default one. A choice that matches no input is
/// [`Error::NoInputDevice`]: the caller decides whether to fall back (R4).
///
/// When the default is a Bluetooth headset and a wired or built-in input
/// exists, the wired one is used: opening a headset's microphone forces the
/// low-rate hands-free profile, which gates the room to silence between
/// words. Only the platform default is replaced; a device the user chooses
/// is respected. This follows anarlog's `active_mic_device`, using the
/// transport cpal reports (WASAPI does; CoreAudio does not yet, so on macOS
/// the default is kept).
pub fn resolve_input(
    host: &cpal::Host,
    choice: Option<&MicChoice>,
    bluetooth: &Mutex<HashMap<String, bool>>,
) -> Result<Device, Error> {
    resolve_input_from(host, choice, bluetooth).map(|(device, _)| device)
}

/// [`resolve_input`], and whether the device is cpal's handle for the
/// platform default (true) or a specific device (false). On Windows the
/// default handle opens in a different way than a device found by its ID
/// (see [`retry_by_id`]).
pub fn resolve_input_from(
    host: &cpal::Host,
    choice: Option<&MicChoice>,
    bluetooth: &Mutex<HashMap<String, bool>>,
) -> Result<(Device, bool), Error> {
    if let Some(choice) = choice {
        let mut devices: Vec<Device> = host
            .input_devices()
            .map_err(|_| Error::MicOpenFailed)?
            .collect();
        let inputs: Vec<Input> = devices.iter().map(input_of).collect();
        let index = find_choice(&inputs, choice).ok_or(Error::NoInputDevice)?;
        return Ok((devices.swap_remove(index), false));
    }
    let default = host.default_input_device().ok_or(Error::NoInputDevice)?;
    // `description()` enumerates the device's configs, which costs more than
    // the whole open; the verdict is remembered per device so only
    // `prepare` pays it.
    let default_key = cache_key(&default);
    let cached = bluetooth
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&default_key)
        .copied();
    let default_is_bluetooth = cached.unwrap_or_else(|| {
        let verdict = is_bluetooth(&default);
        bluetooth
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(default_key, verdict);
        verdict
    });
    if !default_is_bluetooth {
        return Ok((default, true));
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
            Ok((device, false))
        }
        None => Ok((default, true)),
    }
}

/// Whether a failed open tries the default microphone again, opened by its
/// ID (KTD5). Only when the user chose no microphone and the failed device
/// was cpal's default handle.
///
/// On Windows, cpal opens the default handle through
/// `ActivateAudioInterfaceAsync`. After the default device changes during a
/// session, each such open can fail with `RPC_E_CHANGED_MODE` until the app
/// restarts. A device found by its ID opens through `IMMDevice::Activate`
/// and does not have this fault. Shorthand 19029f4 retries in the same way.
pub fn retry_by_id(no_choice: bool, default_handle: bool, failed: bool) -> bool {
    no_choice && default_handle && failed
}

/// The current default input, found by its ID, not as cpal's default
/// handle (see [`retry_by_id`]).
#[cfg(windows)]
pub fn default_by_id(host: &cpal::Host) -> Option<Device> {
    let id = host.default_input_device()?.id().ok()?;
    host.device_by_id(&id)
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

    fn input(id: &str, name: &str) -> Input {
        Input {
            id: id.into(),
            name: name.into(),
        }
    }

    fn choice(id: &str, label: &str) -> MicChoice {
        MicChoice {
            id: id.into(),
            label: label.into(),
        }
    }

    #[test]
    fn a_readable_description_gives_its_name_only() {
        let description = cpal::DeviceDescriptionBuilder::new("MacBook Pro Microphone")
            .manufacturer("Apple Inc.")
            .interface_type(InterfaceType::BuiltIn)
            .build();
        assert_eq!(
            name_or_fallback(Ok::<_, ()>(description)),
            "MacBook Pro Microphone"
        );
    }

    #[test]
    fn an_unreadable_description_gives_the_fallback_name() {
        assert_eq!(name_or_fallback(Err(())), UNKNOWN_DEVICE_NAME);
    }

    #[test]
    fn a_saved_id_matches_an_input_with_the_fallback_name() {
        let devices = [
            input("usb-1", "USB Audio"),
            input("usb-2", UNKNOWN_DEVICE_NAME),
        ];
        assert_eq!(find_choice(&devices, &choice("usb-2", "Old Name")), Some(1));
    }

    #[test]
    fn the_id_wins_over_a_shared_name() {
        let devices = [input("usb-1", "USB Audio"), input("usb-2", "USB Audio")];
        assert_eq!(
            find_choice(&devices, &choice("usb-2", "USB Audio")),
            Some(1)
        );
        assert_eq!(
            find_choice(&devices, &choice("usb-1", "USB Audio")),
            Some(0)
        );
    }

    #[test]
    fn a_missing_id_falls_back_to_the_exact_name() {
        let devices = [
            input("built-in", "MacBook Pro Microphone"),
            input("usb-9", "Yeti"),
        ];
        assert_eq!(find_choice(&devices, &choice("usb-1", "Yeti")), Some(1));
        // The name must match exactly.
        assert_eq!(find_choice(&devices, &choice("usb-1", "yeti")), None);
    }

    #[test]
    fn neither_id_nor_name_is_no_input() {
        let devices = [input("built-in", "MacBook Pro Microphone")];
        assert_eq!(find_choice(&devices, &choice("usb-1", "Yeti")), None);
        assert_eq!(find_choice(&[], &choice("usb-1", "Yeti")), None);
    }

    #[test]
    fn an_empty_id_matches_by_name_only() {
        // A device whose ID could not be read has an empty ID; it must not
        // match a choice that has no ID either.
        let devices = [input("", "Nameless ID"), input("usb-9", "Yeti")];
        assert_eq!(find_choice(&devices, &choice("", "Yeti")), Some(1));
    }

    #[test]
    fn a_failed_default_handle_with_no_choice_is_tried_again_by_id() {
        assert!(retry_by_id(true, true, true));
    }

    #[test]
    fn a_chosen_or_replaced_device_or_a_good_open_is_not_tried_again() {
        // The user chose the microphone.
        assert!(!retry_by_id(false, false, true));
        // A wired input replaced a Bluetooth default.
        assert!(!retry_by_id(true, false, true));
        // The first open worked.
        assert!(!retry_by_id(true, true, false));
    }
}

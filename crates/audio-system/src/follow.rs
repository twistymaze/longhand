//! The Windows Teams mode rule, kept free of WASAPI so that it is tested on
//! every platform. [`TeamsMode`] chooses between per-process capture and
//! device capture for app audio while Teams plays (KTD5).

use std::time::{Duration, Instant};

/// Time between two scans of the processes and output devices.
pub const ENDPOINT_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Time between two reads of the Teams session meters (KTD5).
pub const TEAMS_METER_INTERVAL: Duration = Duration::from_millis(200);

/// How long Teams stays below [`TEAMS_PEAK_THRESHOLD`] before the capture
/// goes back to per-process clients (KTD5).
pub const TEAMS_QUIET_AFTER: Duration = Duration::from_secs(2);

/// The peak meter value of a Teams session above which Teams plays:
/// -60 dBFS.
///
/// Provisional: U1 has not measured Teams yet. The value assumes that an
/// open, idle Teams session shows a peak of 0, and that a call shows peaks
/// far above -60 dBFS, also in its quiet parts. A low value keeps the
/// device capture through the quiet parts of a call, so the mode changes
/// less often. Set the value from U1's measurements.
pub const TEAMS_PEAK_THRESHOLD: f32 = 0.001;

/// The peak meter of one render session that belongs to Teams, and the
/// ID of its output device.
#[derive(Debug, Clone, PartialEq)]
pub struct TeamsReading {
    pub endpoint: String,
    pub peak: f32,
}

/// What the Windows app capture records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureMode {
    /// One per-process loopback client for each root process of the apps
    /// in the set.
    PerProcess,
    /// All sound of the output device with this ID (R19).
    Device(String),
}

/// The Teams mode rule (KTD5, R19). The mode is [`CaptureMode::Device`]
/// from the first Teams reading above [`TEAMS_PEAK_THRESHOLD`] until
/// [`TEAMS_QUIET_AFTER`] passes with no reading above it. The mode is
/// always [`CaptureMode::PerProcess`] when Teams is not in the app set.
#[derive(Debug)]
pub struct TeamsMode {
    mode: CaptureMode,
    last_sound: Option<Instant>,
}

impl Default for TeamsMode {
    fn default() -> Self {
        Self::new()
    }
}

impl TeamsMode {
    pub fn new() -> Self {
        Self {
            mode: CaptureMode::PerProcess,
            last_sound: None,
        }
    }

    /// Take one read of the Teams session meters. Returns the mode.
    pub fn observe(
        &mut self,
        teams_in_set: bool,
        readings: &[TeamsReading],
        now: Instant,
    ) -> &CaptureMode {
        if !teams_in_set {
            self.mode = CaptureMode::PerProcess;
            self.last_sound = None;
            return &self.mode;
        }
        let loudest = readings
            .iter()
            .filter(|r| r.peak > TEAMS_PEAK_THRESHOLD)
            .max_by(|a, b| a.peak.total_cmp(&b.peak));
        if let Some(reading) = loudest {
            self.mode = CaptureMode::Device(reading.endpoint.clone());
            self.last_sound = Some(now);
        } else if self
            .last_sound
            .is_none_or(|t| now.duration_since(t) >= TEAMS_QUIET_AFTER)
        {
            self.mode = CaptureMode::PerProcess;
            self.last_sound = None;
        }
        &self.mode
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reading(endpoint: &str, peak: f32) -> TeamsReading {
        TeamsReading {
            endpoint: endpoint.to_owned(),
            peak,
        }
    }

    fn device(endpoint: &str) -> CaptureMode {
        CaptureMode::Device(endpoint.to_owned())
    }

    fn at(start: Instant, ms: u64) -> Instant {
        start + Duration::from_millis(ms)
    }

    #[test]
    fn the_first_teams_reading_above_the_threshold_starts_device_capture() {
        let start = Instant::now();
        let mut teams = TeamsMode::new();
        let silent = [reading("speakers", 0.0)];
        assert_eq!(
            teams.observe(true, &silent, start),
            &CaptureMode::PerProcess
        );
        let loud = [reading("speakers", 0.0), reading("headset", 0.2)];
        assert_eq!(
            teams.observe(true, &loud, at(start, 200)),
            &device("headset")
        );
    }

    #[test]
    fn device_capture_ends_only_after_2_s_below_the_threshold() {
        let start = Instant::now();
        let mut teams = TeamsMode::new();
        let loud = [reading("headset", 0.2)];
        let silent = [reading("headset", 0.0)];
        assert_eq!(teams.observe(true, &loud, start), &device("headset"));
        for ms in (200..2_000).step_by(200) {
            assert_eq!(
                teams.observe(true, &silent, at(start, ms)),
                &device("headset"),
                "still device capture at {ms} ms"
            );
        }
        assert_eq!(
            teams.observe(true, &silent, at(start, 2_000)),
            &CaptureMode::PerProcess
        );
    }

    #[test]
    fn a_teams_reading_above_the_threshold_restarts_the_2_s() {
        let start = Instant::now();
        let mut teams = TeamsMode::new();
        let loud = [reading("headset", 0.2)];
        let silent = [reading("headset", 0.0)];
        teams.observe(true, &loud, start);
        teams.observe(true, &silent, at(start, 1_800));
        teams.observe(true, &loud, at(start, 1_900));
        assert_eq!(
            teams.observe(true, &silent, at(start, 3_800)),
            &device("headset")
        );
        assert_eq!(
            teams.observe(true, &silent, at(start, 3_900)),
            &CaptureMode::PerProcess
        );
    }

    #[test]
    fn an_open_silent_teams_session_never_starts_device_capture() {
        let start = Instant::now();
        let mut teams = TeamsMode::new();
        let quiet = [reading("speakers", TEAMS_PEAK_THRESHOLD)];
        for ms in (0..60_000).step_by(200) {
            assert_eq!(
                teams.observe(true, &quiet, at(start, ms)),
                &CaptureMode::PerProcess
            );
        }
    }

    #[test]
    fn no_device_capture_when_teams_is_not_in_the_set_also_when_teams_plays() {
        let start = Instant::now();
        let mut teams = TeamsMode::new();
        let loud = [reading("headset", 0.5)];
        for ms in (0..10_000).step_by(200) {
            assert_eq!(
                teams.observe(false, &loud, at(start, ms)),
                &CaptureMode::PerProcess
            );
        }
    }

    #[test]
    fn teams_removed_from_the_set_ends_device_capture_at_once() {
        let start = Instant::now();
        let mut teams = TeamsMode::new();
        let loud = [reading("headset", 0.5)];
        assert_eq!(teams.observe(true, &loud, start), &device("headset"));
        assert_eq!(
            teams.observe(false, &loud, at(start, 200)),
            &CaptureMode::PerProcess
        );
    }

    #[test]
    fn device_capture_follows_teams_to_another_output() {
        let start = Instant::now();
        let mut teams = TeamsMode::new();
        teams.observe(true, &[reading("speakers", 0.3)], start);
        assert_eq!(
            teams.observe(true, &[reading("headset", 0.3)], at(start, 200)),
            &device("headset")
        );
    }
}

//! The Windows output-device rules, kept free of WASAPI so that they are
//! tested on every platform.
//!
//! [`EndpointFollower`] is anarlog's rule (see PROVENANCE.md): capture the
//! default output device, unless another process plays to a different
//! device while the default has no active session, and switch only after
//! two polls agree. The user kept this rule (KTD4). [`OtherOutputWatch`]
//! finds the case that the rule misses: an app holds an open, silent stream
//! on the default while the call plays on another device (U4 step 6).

use std::time::{Duration, Instant};

/// Time between two polls of the output devices.
pub const ENDPOINT_POLL_INTERVAL: Duration = Duration::from_secs(2);

// A meeting app can start rendering to a non-default endpoint after capture began. Requiring two
// consecutive polls keeps a one-off system sound from bouncing the stream.
const ENDPOINT_SWITCH_CONFIRMATIONS: u8 = 2;

// Tracks which render endpoint `open_render_device` would pick and reports a switch once the pick
// has stayed away from the captured endpoint for `ENDPOINT_SWITCH_CONFIRMATIONS` polls.
pub struct EndpointFollower {
    current: String,
    pending: Option<(String, u8)>,
}

impl EndpointFollower {
    pub fn new(current: String) -> Self {
        Self {
            current,
            pending: None,
        }
    }

    pub fn observe(&mut self, observed: String) -> Option<String> {
        if observed == self.current {
            self.pending = None;
            return None;
        }

        let count = match &self.pending {
            Some((id, count)) if *id == observed => count.saturating_add(1),
            _ => 1,
        };

        if count >= ENDPOINT_SWITCH_CONFIRMATIONS {
            self.pending = None;
            Some(observed)
        } else {
            self.pending = Some((observed, count));
            None
        }
    }

    pub fn reset(&mut self) {
        self.pending = None;
    }
}

/// How long the followed output stays silent, while another output plays,
/// before the flag sets.
pub const OTHER_OUTPUT_AFTER: Duration = Duration::from_secs(30);

/// The peak meter value below which an output counts as silent: -60 dBFS.
pub const PEAK_FLOOR: f32 = 0.001;

/// The other-output rule (U4 step 6). The flag sets when the followed
/// output stays silent for [`OTHER_OUTPUT_AFTER`] while another output
/// shows sound. It clears when the followed output shows sound, or when the
/// follower switches to another device ([`OtherOutputWatch::reset`]).
#[derive(Debug, Default)]
pub struct OtherOutputWatch {
    silent_since: Option<Instant>,
    flag: bool,
}

impl OtherOutputWatch {
    pub fn new() -> Self {
        Self::default()
    }

    /// Take one poll: the peak of the followed output and the highest peak
    /// of the other active outputs. Returns the flag.
    pub fn observe(&mut self, followed_peak: f32, other_peak: f32, now: Instant) -> bool {
        if followed_peak >= PEAK_FLOOR {
            self.silent_since = None;
            self.flag = false;
            return false;
        }
        let since = *self.silent_since.get_or_insert(now);
        if other_peak >= PEAK_FLOOR && now.duration_since(since) >= OTHER_OUTPUT_AFTER {
            self.flag = true;
        }
        self.flag
    }

    /// The follower switched to another device.
    pub fn reset(&mut self) {
        self.silent_since = None;
        self.flag = false;
    }

    #[cfg(test)]
    pub fn flag(&self) -> bool {
        self.flag
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_follower_ignores_unchanged_endpoint() {
        let mut follower = EndpointFollower::new("speakers".into());
        assert_eq!(follower.observe("speakers".into()), None);
        assert_eq!(follower.observe("speakers".into()), None);
    }

    #[test]
    fn endpoint_follower_switches_after_consecutive_confirmations() {
        let mut follower = EndpointFollower::new("speakers".into());
        assert_eq!(follower.observe("headset".into()), None);
        assert_eq!(
            follower.observe("headset".into()),
            Some("headset".to_string())
        );
    }

    #[test]
    fn endpoint_follower_resets_on_one_off_change() {
        let mut follower = EndpointFollower::new("speakers".into());
        assert_eq!(follower.observe("headset".into()), None);
        assert_eq!(follower.observe("speakers".into()), None);
        assert_eq!(follower.observe("headset".into()), None);
        assert_eq!(
            follower.observe("headset".into()),
            Some("headset".to_string())
        );
    }

    #[test]
    fn endpoint_follower_restarts_count_when_candidate_changes() {
        let mut follower = EndpointFollower::new("speakers".into());
        assert_eq!(follower.observe("headset".into()), None);
        assert_eq!(follower.observe("monitor".into()), None);
        assert_eq!(
            follower.observe("monitor".into()),
            Some("monitor".to_string())
        );
    }

    #[test]
    fn endpoint_follower_reset_clears_partial_confirmation() {
        let mut follower = EndpointFollower::new("speakers".into());
        assert_eq!(follower.observe("headset".into()), None);
        follower.reset();
        assert_eq!(follower.observe("headset".into()), None);
        assert_eq!(
            follower.observe("headset".into()),
            Some("headset".to_string())
        );
    }

    fn polls(
        watch: &mut OtherOutputWatch,
        start: Instant,
        from: u64,
        to: u64,
        followed: f32,
        other: f32,
    ) -> bool {
        let mut flag = false;
        for s in (from..=to).step_by(2) {
            flag = watch.observe(followed, other, start + Duration::from_secs(s));
        }
        flag
    }

    #[test]
    fn a_silent_followed_output_for_30_s_while_another_plays_sets_the_flag() {
        let start = Instant::now();
        let mut watch = OtherOutputWatch::new();
        assert!(
            !polls(&mut watch, start, 0, 28, 0.0, 0.2),
            "not yet at 28 s"
        );
        assert!(watch.observe(0.0, 0.2, start + Duration::from_secs(30)));
        assert!(watch.flag());
    }

    #[test]
    fn sound_on_the_followed_output_clears_the_flag() {
        let start = Instant::now();
        let mut watch = OtherOutputWatch::new();
        assert!(polls(&mut watch, start, 0, 30, 0.0, 0.2));
        assert!(!watch.observe(0.1, 0.2, start + Duration::from_secs(32)));
        assert!(
            !watch.observe(0.0, 0.2, start + Duration::from_secs(34)),
            "the 30 s count starts again"
        );
    }

    #[test]
    fn no_other_output_with_sound_sets_no_flag() {
        let start = Instant::now();
        let mut watch = OtherOutputWatch::new();
        assert!(!polls(&mut watch, start, 0, 60, 0.0, 0.0));
    }

    #[test]
    fn a_switch_clears_the_flag() {
        let start = Instant::now();
        let mut watch = OtherOutputWatch::new();
        assert!(polls(&mut watch, start, 0, 30, 0.0, 0.2));
        watch.reset();
        assert!(!watch.flag());
        assert!(!watch.observe(0.0, 0.2, start + Duration::from_secs(32)));
    }

    #[test]
    fn sound_on_the_followed_output_during_the_30_s_restarts_the_count() {
        let start = Instant::now();
        let mut watch = OtherOutputWatch::new();
        assert!(!polls(&mut watch, start, 0, 20, 0.0, 0.2));
        assert!(!watch.observe(0.1, 0.2, start + Duration::from_secs(22)));
        assert!(
            !polls(&mut watch, start, 24, 50, 0.0, 0.2),
            "26 s of silence"
        );
        assert!(watch.observe(0.0, 0.2, start + Duration::from_secs(54)));
    }
}

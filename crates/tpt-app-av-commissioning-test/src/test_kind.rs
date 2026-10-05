//! Test-kind taxonomy (§13.1–13.9).
//!
//! Every commissioning test belongs to one of the nine categories the spec
//! distinguishes — connectivity, identity, power, input/output, video, audio,
//! control, synchronisation, and network. Each category has its own measured
//! surface (§13):
//!
//! * *Connectivity* — TCP/UDP reachable, HTTP response, OSC response, MIDI
//!   device present, serial connection available (§13.1).
//! * *Identity* — manufacturer, model, serial, firmware, expected address;
//!   detects wrong devices being installed (§13.2).
//! * *Power* — power state, power on, power off, state feedback, power
//!   recovery (§13.3).
//! * *Input/output* — select input, verify signal arrives, verify expected
//!   output and expected route (§13.4).
//! * *Video* — resolution, frame rate, colour format / space, HDR state,
//!   signal lock, timing, black level, test-pattern response (§13.5).
//! * *Audio* — channel presence, routing, level, silence, frequency
//!   response, polarity, phase, clipping, noise (§13.6).
//! * *Control* — command → device → expected state → feedback (§13.7).
//! * *Synchronisation* — audio/video offset, device timing, clock drift,
//!   multi-device alignment (§13.8).
//! * *Network* — IP, gateway, DNS, latency, packet loss, link state, required
//!   ports, device reachability (§13.9).
//!
//! The kind is what section 31 uses to scatter results by category, and what
//! a suite file can pin with `kind: connectivity`. It never changes *whether*
//! a test runs — locking and dependency rules operate on the generic
//! [`TestRequirements`] a test declares.
//!
//! Licensed under either of MIT OR Apache-2.0, at your option.

use std::fmt;

use serde::{Deserialize, Serialize};

/// The nine commissioning test categories (§13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestKind {
    /// §13.1 — TCP/UDP reachable, HTTP/OSC response, MIDI present, serial up.
    Connectivity,
    /// §13.2 — manufacturer, model, serial, firmware, expected address.
    Identity,
    /// §13.3 — power state, on/off, state feedback, power recovery.
    Power,
    /// §13.4 — select input, verify signal, verify output/route.
    InputOutput,
    /// §13.5 — resolution, frame rate, colour format/space, HDR, sync, timing.
    Video,
    /// §13.6 — channel presence, routing, level, silence, response, polarity.
    Audio,
    /// §13.7 — command → device → expected state → feedback.
    Control,
    /// §13.8 — audio/video offset, timing, clock drift, multi-device sync.
    Synchronization,
    /// §13.9 — IP, gateway, DNS, latency, packet loss, link, ports, reachable.
    Network,
}

impl TestKind {
    /// The nine kinds, in spec order (§13.1–13.9).
    pub const ALL: [TestKind; 9] = [
        TestKind::Connectivity,
        TestKind::Identity,
        TestKind::Power,
        TestKind::InputOutput,
        TestKind::Video,
        TestKind::Audio,
        TestKind::Control,
        TestKind::Synchronization,
        TestKind::Network,
    ];

    /// Stable lowercase name (mirrors the serialized form).
    pub fn as_str(&self) -> &'static str {
        match self {
            TestKind::Connectivity => "connectivity",
            TestKind::Identity => "identity",
            TestKind::Power => "power",
            TestKind::InputOutput => "input_output",
            TestKind::Video => "video",
            TestKind::Audio => "audio",
            TestKind::Control => "control",
            TestKind::Synchronization => "synchronization",
            TestKind::Network => "network",
        }
    }
}

impl fmt::Display for TestKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Classify a measured field name onto a test kind (§13).
///
/// This is the *model* mapping: a field like `resolution` is video; `level`
/// is audio; `latency` is network. Unknown fields are not guessed — they
/// return `None` so an unrecognised measurement stays uncategorized rather
/// than being silently forced into the wrong kind.
pub fn classify_field(field: &str) -> Option<TestKind> {
    let f = field.trim();
    if f.is_empty() {
        return None;
    }
    Some(match f {
        // §13.1 connectivity
        "tcp_reachable" | "udp_reachable" | "http_response" | "osc_response" | "midi_present"
        | "serial_available" | "reachable" => TestKind::Connectivity,
        // §13.2 identity
        "manufacturer" | "model" | "serial" | "firmware" | "expected_address"
        | "device_address" => TestKind::Identity,
        // §13.3 power
        "power_state" | "power_on" | "power_off" | "power_feedback" | "power_recovery"
        | "state_feedback" => TestKind::Power,
        // §13.4 input/output
        "input_selected" | "output_selected" | "signal_present" | "expected_route"
        | "selected_input" | "selected_output" => TestKind::InputOutput,
        // §13.5 video
        "resolution"
        | "frame_rate"
        | "colour_format"
        | "colour_space"
        | "hdr_state"
        | "signal_lock"
        | "signal_timing"
        | "black_level"
        | "test_pattern_response"
        | "video_resolution"
        | "video_frame_rate" => TestKind::Video,
        // §13.6 audio
        "channel_presence" | "routing" | "level" | "silence" | "frequency_response"
        | "polarity" | "phase" | "clipping" | "noise" | "audio_level" | "channel_map" => {
            TestKind::Audio
        }
        // §13.7 control
        "command_feedback" | "control_response" | "state_after_command" => TestKind::Control,
        // §13.8 synchronisation
        "audio_video_offset"
        | "av_offset"
        | "device_timing"
        | "clock_drift"
        | "cross_device_offset"
        | "sync_offset" => TestKind::Synchronization,
        // §13.9 network
        "ip_address" | "gateway" | "dns" | "latency" | "packet_loss" | "link_state"
        | "required_ports" | "mtu" => TestKind::Network,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status::TestStatus;

    #[allow(dead_code)]
    fn assert_status_import_is_reachable() -> TestStatus {
        TestStatus::Pass
    }

    #[test]
    fn all_kinds_exist_in_spec_order() {
        assert_eq!(TestKind::ALL.len(), 9);
        assert_eq!(TestKind::ALL[0], TestKind::Connectivity);
        assert_eq!(TestKind::ALL[4], TestKind::Video);
        assert_eq!(TestKind::ALL[8], TestKind::Network);
    }

    #[test]
    fn every_kind_has_a_stable_name() {
        for kind in TestKind::ALL {
            assert!(!kind.as_str().is_empty());
            assert_eq!(kind.as_str(), format!("{kind}"));
        }
    }

    #[test]
    fn kind_names_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for kind in TestKind::ALL {
            assert!(seen.insert(kind.as_str()));
        }
    }

    #[test]
    fn classified_fields_map_to_video() {
        assert_eq!(classify_field("resolution"), Some(TestKind::Video));
        assert_eq!(classify_field("frame_rate"), Some(TestKind::Video));
        assert_eq!(classify_field("hdr_state"), Some(TestKind::Video));
    }

    #[test]
    fn classified_fields_map_to_audio() {
        assert_eq!(classify_field("level"), Some(TestKind::Audio));
        assert_eq!(classify_field("polarity"), Some(TestKind::Audio));
        assert_eq!(classify_field("silence"), Some(TestKind::Audio));
    }

    #[test]
    fn classified_fields_map_to_network() {
        assert_eq!(classify_field("latency"), Some(TestKind::Network));
        assert_eq!(classify_field("packet_loss"), Some(TestKind::Network));
        assert_eq!(classify_field("ip_address"), Some(TestKind::Network));
    }

    #[test]
    fn classification_covers_the_power_and_io_surface() {
        assert_eq!(classify_field("power_state"), Some(TestKind::Power));
        assert_eq!(classify_field("power_on"), Some(TestKind::Power));
        assert_eq!(
            classify_field("selected_input"),
            Some(TestKind::InputOutput)
        );
        assert_eq!(
            classify_field("expected_route"),
            Some(TestKind::InputOutput)
        );
    }

    #[test]
    fn unknown_fields_are_not_guessed() {
        assert_eq!(classify_field("make"), None);
        assert_eq!(classify_field(""), None);
        assert_eq!(classify_field("  "), None);
    }
}

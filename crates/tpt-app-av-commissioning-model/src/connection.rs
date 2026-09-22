//! Connection model (§8): `Connection`, `SignalType`, `Transport`,
//! `ConnectionExpectation`.

use serde::{Deserialize, Serialize};

use crate::id::{ConnectionId, EndpointId};

/// What a signal is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalType {
    Video,
    Audio,
    AudioVideo,
    Network,
    Control,
    Lighting,
    Clock,
    Unknown,
}

/// How a signal is carried.
///
/// Extensible: `Other` covers proprietary and emerging protocols.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    Hdmi,
    Sdi,
    DisplayPort,
    Usb,
    Aes3,
    Analog,
    Dante,
    Ndi,
    Rtp,
    Rtsp,
    Osc,
    Midi,
    ArtNet,
    Sacn,
    Ethernet,
    Serial,
    Other,
}

impl Transport {
    /// Stable lowercase name.
    pub fn as_str(&self) -> &'static str {
        match self {
            Transport::Hdmi => "hdmi",
            Transport::Sdi => "sdi",
            Transport::DisplayPort => "display_port",
            Transport::Usb => "usb",
            Transport::Aes3 => "aes3",
            Transport::Analog => "analog",
            Transport::Dante => "dante",
            Transport::Ndi => "ndi",
            Transport::Rtp => "rtp",
            Transport::Rtsp => "rtsp",
            Transport::Osc => "osc",
            Transport::Midi => "midi",
            Transport::ArtNet => "artnet",
            Transport::Sacn => "sacn",
            Transport::Ethernet => "ethernet",
            Transport::Serial => "serial",
            Transport::Other => "other",
        }
    }
}

/// A known relationship between two endpoints.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Connection {
    pub id: ConnectionId,
    pub source: EndpointId,
    pub destination: EndpointId,
    pub signal_type: SignalType,
    pub transport: Transport,
    pub expected: ConnectionExpectation,
}

impl Connection {
    /// Create a new connection with default expectations.
    pub fn new(
        id: ConnectionId,
        source: EndpointId,
        destination: EndpointId,
        signal_type: SignalType,
        transport: Transport,
    ) -> Self {
        Self {
            id,
            source,
            destination,
            signal_type,
            transport,
            expected: ConnectionExpectation::default(),
        }
    }
}

/// What "working" means for a connection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectionExpectation {
    /// Whether a signal is expected to be present at the destination.
    pub signal_present: bool,
    /// Minimum acceptable bit rate in bits per second (network transports).
    pub min_bit_rate_bps: Option<u64>,
    /// Expected number of audio channels (audio/AV transports).
    pub audio_channels: Option<u32>,
    /// Maximum acceptable latency in milliseconds.
    pub max_latency_ms: Option<u32>,
    /// Expected video format at the destination.
    pub video: Option<VideoRequirement>,
    /// Human-visible expectation note shown to engineers.
    pub note: Option<String>,
}

impl Default for ConnectionExpectation {
    fn default() -> Self {
        Self {
            signal_present: true,
            min_bit_rate_bps: None,
            audio_channels: None,
            max_latency_ms: None,
            video: None,
            note: None,
        }
    }
}

/// A video format requirement (resolution / frame rate / HDR).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VideoRequirement {
    pub resolution: Option<String>,
    pub frame_rate: Option<f64>,
    pub hdr: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::EndpointId;

    fn eps() -> (EndpointId, EndpointId) {
        (EndpointId::new("src-out"), EndpointId::new("dst-in"))
    }

    #[test]
    fn connection_default_expects_signal() {
        let (s, d) = eps();
        let c = Connection::new(
            ConnectionId::new("c1"),
            s,
            d,
            SignalType::AudioVideo,
            Transport::Hdmi,
        );
        assert!(c.expected.signal_present);
        assert_eq!(c.expected.audio_channels, None);
    }

    #[test]
    fn transport_and_signal_string_forms() {
        assert_eq!(Transport::DisplayPort.as_str(), "display_port");
        assert_eq!(serde_json::to_string(&SignalType::AudioVideo).unwrap(), "\"audio_video\"");
        assert_eq!(serde_json::to_string(&Transport::Sacn).unwrap(), "\"sacn\"");
    }

    #[test]
    fn expectation_serde_round_trip() {
        let e = ConnectionExpectation {
            audio_channels: Some(8),
            max_latency_ms: Some(100),
            video: Some(VideoRequirement {
                resolution: Some("3840x2160".to_owned()),
                frame_rate: Some(60.0),
                hdr: Some(false),
            }),
            ..Default::default()
        };
        let json = serde_json::to_string(&e).unwrap();
        let back: ConnectionExpectation = serde_json::from_str(&json).unwrap();
        assert_eq!(back, e);
    }
}
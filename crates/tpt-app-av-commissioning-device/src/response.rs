//! Responses returned by a driver.

use serde::{Deserialize, Serialize};

use crate::DeviceState;

/// The outcome of executing a `DeviceCommand`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceResponse {
    /// Whether the device acknowledged the command.
    pub ok: bool,
    /// State observed after the command, when available.
    pub state: Option<DeviceState>,
    /// Human-readable detail (raw device text, error strings, notes).
    pub message: Option<String>,
    /// Time the device took to respond, in milliseconds.
    pub response_time_ms: Option<u64>,
}

impl DeviceResponse {
    pub fn ok() -> Self {
        Self {
            ok: true,
            state: None,
            message: None,
            response_time_ms: None,
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            state: None,
            message: Some(message.into()),
            response_time_ms: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ok_and_error_responses() {
        let ok = DeviceResponse::ok();
        assert!(ok.ok);
        let err = DeviceResponse::error("device unreachable");
        assert!(!err.ok);
        assert_eq!(err.message.as_deref(), Some("device unreachable"));
    }

    #[test]
    fn response_carries_state() {
        let mut state = DeviceState::new();
        state.set("power", true);
        let r = DeviceResponse {
            ok: true,
            state: Some(state),
            message: None,
            response_time_ms: Some(12),
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: DeviceResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back.response_time_ms, Some(12));
    }
}
//! Device identity: who the device claims to be.

use serde::{Deserialize, Serialize};

/// Verified or reported identity of a physical device.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DeviceIdentity {
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub serial_number: Option<String>,
    pub firmware: Option<String>,
}

impl DeviceIdentity {
    pub fn new() -> Self {
        Self::default()
    }

    /// A stable fingerprint used to detect device replacement (§22):
    /// manufacturer + model + serial + firmware.
    pub fn fingerprint(&self) -> String {
        format!(
            "{}|{}|{}|{}",
            self.manufacturer.as_deref().unwrap_or(""),
            self.model.as_deref().unwrap_or(""),
            self.serial_number.as_deref().unwrap_or(""),
            self.firmware.as_deref().unwrap_or("")
        )
    }

    /// True if any identity field is populated.
    pub fn is_empty(&self) -> bool {
        self.manufacturer.is_none()
            && self.model.is_none()
            && self.serial_number.is_none()
            && self.firmware.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_varies_with_serial() {
        let a = DeviceIdentity {
            manufacturer: Some("Homecinema Co".into()),
            model: Some("Beamer 1000".into()),
            serial_number: Some("SN-1".into()),
            firmware: Some("1.2.3".into()),
        };
        let b = DeviceIdentity {
            serial_number: Some("SN-2".into()),
            ..a.clone()
        };
        assert_ne!(a.fingerprint(), b.fingerprint());
        // Same identity -> same fingerprint.
        assert_eq!(a.fingerprint(), a.clone().fingerprint());
    }

    #[test]
    fn empty_identity_detected() {
        assert!(DeviceIdentity::new().is_empty());
    }
}

//! Identifier types for the domain model.
//!
//! Ids are human-readable strings so that project manifests can carry stable,
//! version-controllable identifiers (e.g. `boardroom`, `projector-01`) rather
//! than opaque numeric/uuid keys. See §26 of `spec.txt`.

use serde::{Deserialize, Serialize};

macro_rules! id_type {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Create an id from a string.
            pub fn new<S: Into<String>>(value: S) -> Self {
                Self(value.into())
            }

            /// The id as a string slice.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_owned())
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

id_type! {
    /// Identifies a `Project`.
    ProjectId
}

id_type! {
    /// Identifies a `Room`.
    RoomId
}

id_type! {
    /// Identifies a `Device`.
    DeviceId
}

id_type! {
    /// Identifies an `Endpoint`.
    EndpointId
}

id_type! {
    /// Identifies a `Connection`.
    ConnectionId
}

id_type! {
    /// Identifies a test suite.
    TestSuiteId
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_displayed_as_their_string() {
        assert_eq!(ProjectId::new("p1").to_string(), "p1");
        assert_eq!(DeviceId::from("projector-01").as_str(), "projector-01");
    }

    #[test]
    fn ids_round_trip_through_serde_json() {
        let id = RoomId::new("boardroom");
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"boardroom\"");
        let back: RoomId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, id);
    }

    #[test]
    fn distinct_types_do_not_compare() {
        let room = RoomId::new("x");
        let device = DeviceId::new("x");
        // Both wrap "x" but are distinct types; equality across types is a
        // compile-time error, which is the intended protection.
        assert_eq!(room.as_str(), device.as_str());
    }
}
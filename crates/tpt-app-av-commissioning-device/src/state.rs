//! Device state: what a device reports about itself.

use serde::{Deserialize, Serialize};

/// A typed key/value state read from a device.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceState {
    /// Ordered state fields, e.g. `power`, `input`, `signal_status`, `edid`.
    pub fields: Vec<StateField>,
}

impl DeviceState {
    pub fn new() -> Self {
        Self { fields: Vec::new() }
    }

    /// Insert or replace a state field.
    pub fn set(&mut self, name: impl Into<String>, value: impl Into<StateValue>) {
        let name = name.into();
        let value = value.into();
        if let Some(field) = self.fields.iter_mut().find(|f| f.name == name) {
            field.value = value;
        } else {
            self.fields.push(StateField { name, value });
        }
    }

    /// Look up a field's value.
    pub fn get(&self, name: &str) -> Option<&StateValue> {
        self.fields
            .iter()
            .find(|f| f.name == name)
            .map(|f| &f.value)
    }

    /// True if the state carries no fields.
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }
}

impl Default for DeviceState {
    fn default() -> Self {
        Self::new()
    }
}

/// A single named state field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StateField {
    pub name: String,
    pub value: StateValue,
}

/// A state value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum StateValue {
    Boolean(bool),
    Integer(i64),
    Float(f64),
    Text(String),
    /// A structured value, e.g. an EDID block or a configuration blob.
    Blob(String),
}

impl From<bool> for StateValue {
    fn from(v: bool) -> Self {
        StateValue::Boolean(v)
    }
}
impl From<i64> for StateValue {
    fn from(v: i64) -> Self {
        StateValue::Integer(v)
    }
}
impl From<f64> for StateValue {
    fn from(v: f64) -> Self {
        StateValue::Float(v)
    }
}
impl From<&str> for StateValue {
    fn from(v: &str) -> Self {
        StateValue::Text(v.to_owned())
    }
}
impl From<String> for StateValue {
    fn from(v: String) -> Self {
        StateValue::Text(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_set_and_get() {
        let mut s = DeviceState::new();
        s.set("power", false);
        s.set("input", "hdmi3".to_owned());
        assert_eq!(s.get("power"), Some(&StateValue::Boolean(false)));
        assert!(s.get("nope").is_none());
    }

    #[test]
    fn state_set_replaces() {
        let mut s = DeviceState::new();
        s.set("volume", 5);
        s.set("volume", 8);
        assert_eq!(s.get("volume"), Some(&StateValue::Integer(8)));
        assert_eq!(s.fields.len(), 1);
    }
}

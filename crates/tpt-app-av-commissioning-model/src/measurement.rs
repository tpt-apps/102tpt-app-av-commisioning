//! Measurement model (§19): preserve raw values, no internal rounding before
//! tolerance evaluation.

use serde::{Deserialize, Serialize};

/// A raw measured value. Variants exist so numeric precision is preserved
/// until tolerance evaluation (no internal rounding).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MeasurementValue {
    Integer(i64),
    Float(f64),
    Text(String),
    Boolean(bool),
}

impl MeasurementValue {
    /// The value as `f64` where it is numeric.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            MeasurementValue::Integer(i) => Some(*i as f64),
            MeasurementValue::Float(f) => Some(*f),
            MeasurementValue::Text(_) | MeasurementValue::Boolean(_) => None,
        }
    }

    /// The value as a human-readable string.
    pub fn display(&self) -> String {
        match self {
            MeasurementValue::Integer(i) => i.to_string(),
            MeasurementValue::Float(f) => format!("{}", f),
            MeasurementValue::Text(t) => t.clone(),
            MeasurementValue::Boolean(b) => b.to_string(),
        }
    }
}

impl From<i64> for MeasurementValue {
    fn from(v: i64) -> Self {
        MeasurementValue::Integer(v)
    }
}
impl From<f64> for MeasurementValue {
    fn from(v: f64) -> Self {
        MeasurementValue::Float(v)
    }
}
impl From<bool> for MeasurementValue {
    fn from(v: bool) -> Self {
        MeasurementValue::Boolean(v)
    }
}

/// A measurement unit. Extensible by construction. Common units are provided
/// as `&'static str` constants; arbitrary units are owned strings.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Unit(std::borrow::Cow<'static, str>);

impl Unit {
    pub const HZ: Unit = Unit(std::borrow::Cow::Borrowed("Hz"));
    pub const KHZ: Unit = Unit(std::borrow::Cow::Borrowed("kHz"));
    pub const DB: Unit = Unit(std::borrow::Cow::Borrowed("dB"));
    pub const DBFS: Unit = Unit(std::borrow::Cow::Borrowed("dBFS"));
    pub const DBM: Unit = Unit(std::borrow::Cow::Borrowed("dBm"));
    pub const MS: Unit = Unit(std::borrow::Cow::Borrowed("ms"));
    pub const SECONDS: Unit = Unit(std::borrow::Cow::Borrowed("s"));
    pub const VOLTS: Unit = Unit(std::borrow::Cow::Borrowed("V"));
    pub const WATTS: Unit = Unit(std::borrow::Cow::Borrowed("W"));
    pub const PERCENT: Unit = Unit(std::borrow::Cow::Borrowed("%"));
    pub const FRAMES: Unit = Unit(std::borrow::Cow::Borrowed("frames"));
    pub const BPS: Unit = Unit(std::borrow::Cow::Borrowed("bps"));
    pub const MBPS: Unit = Unit(std::borrow::Cow::Borrowed("Mbps"));

    /// Create a custom unit string.
    pub fn new<S: Into<String>>(value: S) -> Self {
        Unit(std::borrow::Cow::Owned(value.into()))
    }

    /// The unit as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&'static str> for Unit {
    fn from(value: &'static str) -> Self {
        Unit(std::borrow::Cow::Borrowed(value))
    }
}

impl From<String> for Unit {
    fn from(value: String) -> Self {
        Unit(std::borrow::Cow::Owned(value))
    }
}

/// Where a measurement came from. Used by reports to avoid implying a
/// measurement that did not occur.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementSource {
    DeviceResponse,
    Instrument,
    Computed,
    ManualEntry,
    TestFixture,
    Derived,
}

/// How a measured value is compared to an expected value within tolerance.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Tolerance {
    /// Measured value must equal the reference exactly.
    #[default]
    Exact,
    /// Within an absolute band around the reference.
    Absolute { delta: f64 },
    /// Within a percentage of the reference.
    Percent { percent: f64 },
}

impl Tolerance {
    /// Evaluate `value` against `reference` under this tolerance.
    ///
    /// Returns `None` when either side is not numeric (comparison not
    /// applicable). Floating point equality uses a small epsilon.
    pub fn check(&self, value: &MeasurementValue, reference: &MeasurementValue) -> Option<bool> {
        let value = value.as_f64()?;
        let reference = reference.as_f64()?;
        let eps = 1e-9;
        Some(match self {
            Tolerance::Exact => (value - reference).abs() <= eps,
            Tolerance::Absolute { delta } => (value - reference).abs() <= delta.abs(),
            Tolerance::Percent { percent } => {
                if reference.abs() <= eps {
                    // Division by zero is undefined; only exact zero matches.
                    value.abs() <= eps
                } else {
                    (value - reference).abs() / reference.abs() * 100.0 <= percent.abs()
                }
            }
        })
    }
}

/// A single measurement taken during a test.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Measurement {
    pub name: String,
    pub value: MeasurementValue,
    pub unit: Option<Unit>,
    pub tolerance: Tolerance,
    /// When present, `value` is compared against this expected value.
    pub expected: Option<MeasurementValue>,
    pub source: MeasurementSource,
}

impl Measurement {
    /// A plain measurement with no comparison.
    pub fn new(name: impl Into<String>, value: impl Into<MeasurementValue>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
            unit: None,
            tolerance: Tolerance::Exact,
            expected: None,
            source: MeasurementSource::DeviceResponse,
        }
    }

    /// Evaluate the comparison, if an expected value is set.
    pub fn passes(&self) -> Option<bool> {
        self.tolerance.check(&self.value, self.expected.as_ref()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_integers_are_not_rounded() {
        let m = MeasurementValue::Float(12.345678910);
        assert_eq!(m.as_f64(), Some(12.345678910));
    }

    #[test]
    fn absolute_tolerance() {
        let value = 100.4;
        let expected = 100.0;
        let tol = Tolerance::Absolute { delta: 0.5 };
        assert_eq!(tol.check(&value.into(), &expected.into()), Some(true));
        let too_far = 100.6;
        assert_eq!(tol.check(&too_far.into(), &expected.into()), Some(false));
    }

    #[test]
    fn percent_tolerance() {
        let tol = Tolerance::Percent { percent: 10.0 };
        assert_eq!(tol.check(&101.0.into(), &100.0.into()), Some(true));
        assert_eq!(tol.check(&112.0.into(), &100.0.into()), Some(false));
    }

    #[test]
    fn zero_reference_guard() {
        let tol = Tolerance::Percent { percent: 10.0 };
        assert_eq!(tol.check(&0.0.into(), &0.0.into()), Some(true));
        assert_eq!(tol.check(&1.0.into(), &0.0.into()), Some(false));
    }

    #[test]
    fn non_numeric_comparison_is_not_applicable() {
        let tol = Tolerance::Absolute { delta: 1.0 };
        let text = MeasurementValue::Text("n/a".to_owned());
        assert_eq!(tol.check(&text, &1.0.into()), None);
    }

    #[test]
    fn measurement_passes_only_with_expected() {
        let m = Measurement::new("level", 104.0);
        assert_eq!(m.passes(), None);

        let mut m = Measurement::new("level", 104.0);
        m.unit = Some(Unit::DB);
        m.tolerance = Tolerance::Absolute { delta: 5.0 };
        m.expected = Some(100.0.into());
        assert_eq!(m.passes(), Some(true));
    }
}

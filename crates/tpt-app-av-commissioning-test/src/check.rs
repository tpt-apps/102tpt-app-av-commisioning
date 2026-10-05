//! Expectation engine shared by the concrete test types (§13).
//!
//! An [`Expectation`] names a state field a device reports and the condition
//! it must satisfy. [`evaluate`] checks a [`DeviceState`] against a list of
//! expectations and produces the raw [`Measurement`]s, human-readable messages
//! and the aggregate [`TestStatus`]. Raw values are preserved; tolerance is
//! applied only at comparison time (§19).

use tpt_app_av_commissioning_device::{DeviceState, StateValue};
use tpt_app_av_commissioning_model::{
    Measurement, MeasurementSource, MeasurementValue, Tolerance, Unit,
};

use crate::status::TestStatus;

/// The condition a reported value must satisfy.
#[derive(Debug, Clone, PartialEq)]
pub enum Check {
    /// Equal to `expected` (numbers within `tolerance`, text case-insensitive).
    Equals {
        expected: MeasurementValue,
        tolerance: Tolerance,
    },
    /// Numeric value no greater than the limit.
    AtMost(f64),
    /// Numeric value no less than the limit.
    AtLeast(f64),
    /// Numeric value within the inclusive range.
    Between { min: f64, max: f64 },
}

/// One field a device must report, and what it must read.
#[derive(Debug, Clone, PartialEq)]
pub struct Expectation {
    pub field: String,
    pub check: Check,
    pub unit: Option<Unit>,
    /// A missed advisory expectation yields `Warning` rather than `Fail`.
    pub advisory: bool,
}

impl Expectation {
    fn new(field: impl Into<String>, check: Check) -> Self {
        Self {
            field: field.into(),
            check,
            unit: None,
            advisory: false,
        }
    }

    /// Text field equal to `expected` (trimmed, case-insensitive).
    pub fn text(field: impl Into<String>, expected: impl Into<String>) -> Self {
        Self::new(
            field,
            Check::Equals {
                expected: MeasurementValue::Text(expected.into()),
                tolerance: Tolerance::Exact,
            },
        )
    }

    /// Boolean field equal to `expected`.
    pub fn boolean(field: impl Into<String>, expected: bool) -> Self {
        Self::new(
            field,
            Check::Equals {
                expected: MeasurementValue::Boolean(expected),
                tolerance: Tolerance::Exact,
            },
        )
    }

    /// Numeric field equal to `expected` within `tolerance`.
    pub fn number(field: impl Into<String>, expected: f64, tolerance: Tolerance) -> Self {
        Self::new(
            field,
            Check::Equals {
                expected: MeasurementValue::Float(expected),
                tolerance,
            },
        )
    }

    /// Numeric field no greater than `max`.
    pub fn at_most(field: impl Into<String>, max: f64) -> Self {
        Self::new(field, Check::AtMost(max))
    }

    /// Numeric field no less than `min`.
    pub fn at_least(field: impl Into<String>, min: f64) -> Self {
        Self::new(field, Check::AtLeast(min))
    }

    /// Numeric field within `[min, max]`.
    pub fn between(field: impl Into<String>, min: f64, max: f64) -> Self {
        Self::new(field, Check::Between { min, max })
    }

    /// Record the unit the field is measured in.
    pub fn unit(mut self, unit: Unit) -> Self {
        self.unit = Some(unit);
        self
    }

    /// Downgrade a miss from `Fail` to `Warning`.
    pub fn advisory(mut self) -> Self {
        self.advisory = true;
        self
    }
}

/// The outcome of evaluating a state against expectations.
#[derive(Debug, Clone, PartialEq)]
pub struct Evaluation {
    pub status: TestStatus,
    pub measurements: Vec<Measurement>,
    pub messages: Vec<String>,
}

enum Verdict {
    Met,
    Unmet(String),
}

/// Convert a reported state value into a measurement value (no rounding).
pub fn measurement_value(value: &StateValue) -> MeasurementValue {
    match value {
        StateValue::Boolean(b) => MeasurementValue::Boolean(*b),
        StateValue::Integer(i) => MeasurementValue::Integer(*i),
        StateValue::Float(f) => MeasurementValue::Float(*f),
        StateValue::Text(t) | StateValue::Blob(t) => MeasurementValue::Text(t.clone()),
    }
}

fn judge(actual: &MeasurementValue, check: &Check) -> Verdict {
    match check {
        Check::Equals {
            expected,
            tolerance,
        } => {
            let met = match (actual, expected) {
                (MeasurementValue::Text(a), MeasurementValue::Text(e)) => {
                    a.trim().eq_ignore_ascii_case(e.trim())
                }
                (MeasurementValue::Boolean(a), MeasurementValue::Boolean(e)) => a == e,
                _ => match tolerance.check(actual, expected) {
                    Some(met) => met,
                    None => {
                        return Verdict::Unmet(format!(
                            "expected {}, got {} (type mismatch)",
                            expected.display(),
                            actual.display()
                        ))
                    }
                },
            };
            if met {
                Verdict::Met
            } else {
                Verdict::Unmet(format!(
                    "expected {}, got {}",
                    expected.display(),
                    actual.display()
                ))
            }
        }
        Check::AtMost(max) => numeric(actual, |v| v <= *max, format!("at most {max}")),
        Check::AtLeast(min) => numeric(actual, |v| v >= *min, format!("at least {min}")),
        Check::Between { min, max } => numeric(
            actual,
            |v| v >= *min && v <= *max,
            format!("between {min} and {max}"),
        ),
    }
}

fn numeric(actual: &MeasurementValue, ok: impl Fn(f64) -> bool, want: String) -> Verdict {
    match actual.as_f64() {
        Some(v) if v.is_nan() => Verdict::Unmet(format!("expected {want}, got NaN")),
        Some(v) if ok(v) => Verdict::Met,
        Some(v) => Verdict::Unmet(format!("expected {want}, got {v}")),
        None => Verdict::Unmet(format!(
            "expected {want}, got non-numeric {}",
            actual.display()
        )),
    }
}

/// Evaluate `state` against `expectations`.
///
/// Status precedence: any missed required expectation is `Fail`; otherwise a
/// field the device did not report is `Inconclusive` (the test could not
/// conclude); otherwise a missed advisory expectation is `Warning`; otherwise
/// `Pass`. An empty expectation list is `Inconclusive` — a test that checks
/// nothing proves nothing.
pub fn evaluate(state: &DeviceState, expectations: &[Expectation]) -> Evaluation {
    if expectations.is_empty() {
        return Evaluation {
            status: TestStatus::Inconclusive,
            measurements: Vec::new(),
            messages: vec!["no expectations declared; nothing was verified".to_owned()],
        };
    }

    let mut measurements = Vec::new();
    let mut messages = Vec::new();
    let (mut failed, mut missing, mut warned) = (false, false, false);

    for exp in expectations {
        let Some(value) = state.get(&exp.field) else {
            messages.push(format!("{}: device did not report this field", exp.field));
            if !exp.advisory {
                missing = true;
            }
            continue;
        };
        let actual = measurement_value(value);
        let mut m = Measurement::new(exp.field.clone(), actual.clone());
        m.unit = exp.unit.clone();
        m.source = MeasurementSource::DeviceResponse;
        if let Check::Equals {
            expected,
            tolerance,
        } = &exp.check
        {
            m.expected = Some(expected.clone());
            m.tolerance = *tolerance;
        }
        measurements.push(m);

        match judge(&actual, &exp.check) {
            Verdict::Met => messages.push(format!("{}: {} ok", exp.field, actual.display())),
            Verdict::Unmet(why) => {
                messages.push(format!("{}: {why}", exp.field));
                if exp.advisory {
                    warned = true;
                } else {
                    failed = true;
                }
            }
        }
    }

    let status = if failed {
        TestStatus::Fail
    } else if missing {
        TestStatus::Inconclusive
    } else if warned {
        TestStatus::Warning
    } else {
        TestStatus::Pass
    };
    Evaluation {
        status,
        measurements,
        messages,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> DeviceState {
        let mut s = DeviceState::new();
        s.set("resolution", "3840x2160");
        s.set("frame_rate", 60);
        s.set("latency_ms", 18.5);
        s.set("signal_lock", true);
        s
    }

    #[test]
    fn all_met_passes_with_raw_measurements() {
        let ev = evaluate(
            &state(),
            &[
                Expectation::text("resolution", "3840X2160"),
                Expectation::number("frame_rate", 60.0, Tolerance::Exact),
                Expectation::at_most("latency_ms", 20.0).unit(Unit::MS),
                Expectation::boolean("signal_lock", true),
            ],
        );
        assert_eq!(ev.status, TestStatus::Pass);
        assert_eq!(ev.measurements.len(), 4);
        // Raw value preserved, not rounded.
        assert_eq!(ev.measurements[2].value, MeasurementValue::Float(18.5));
    }

    #[test]
    fn mismatch_fails_and_beats_missing() {
        let ev = evaluate(
            &state(),
            &[
                Expectation::text("resolution", "1920x1080"),
                Expectation::text("colour_format", "rgb"),
            ],
        );
        assert_eq!(ev.status, TestStatus::Fail);
    }

    #[test]
    fn missing_field_is_inconclusive_not_fail() {
        let ev = evaluate(&state(), &[Expectation::text("hdr_state", "off")]);
        assert_eq!(ev.status, TestStatus::Inconclusive);
        assert!(ev.measurements.is_empty());
    }

    #[test]
    fn advisory_miss_is_warning() {
        let ev = evaluate(
            &state(),
            &[
                Expectation::text("resolution", "3840x2160"),
                Expectation::at_most("latency_ms", 10.0).advisory(),
            ],
        );
        assert_eq!(ev.status, TestStatus::Warning);
    }

    #[test]
    fn tolerance_and_range_checks() {
        let ev = evaluate(
            &state(),
            &[
                Expectation::number("latency_ms", 20.0, Tolerance::Absolute { delta: 2.0 }),
                Expectation::between("frame_rate", 59.0, 61.0),
                Expectation::at_least("frame_rate", 60.0),
            ],
        );
        assert_eq!(ev.status, TestStatus::Pass);
    }

    #[test]
    fn type_mismatch_fails() {
        let ev = evaluate(&state(), &[Expectation::at_most("resolution", 5.0)]);
        assert_eq!(ev.status, TestStatus::Fail);
        let ev = evaluate(
            &state(),
            &[Expectation::number("resolution", 1.0, Tolerance::Exact)],
        );
        assert_eq!(ev.status, TestStatus::Fail);
    }

    #[test]
    fn empty_expectations_are_inconclusive() {
        assert_eq!(evaluate(&state(), &[]).status, TestStatus::Inconclusive);
    }
}

//! Audio test-tone generation, WAV I/O and analysis (§13.6, §19).
//!
//! The measurement side of the audio commissioning story, built on the TPT
//! ecosystem: tones are generated with `tpt-dsp-audio`'s oscillator, written
//! and read as 16-bit PCM WAV via `tpt-av-cadence-wav`, and analysed with
//! `tpt-dsp-core` (FFT with a Hann window) and `tpt-dsp-analysis` (RMS,
//! zero-crossing rate). Everything here works on plain sample buffers, so it
//! is fully testable without audio hardware; live capture through
//! `tpt-av-audio-io` feeds the same functions.

use std::fs::File;
use std::path::Path;

use num_complex::Complex;
use tpt_app_av_commissioning_model::{
    Measurement, MeasurementSource, MeasurementValue, Tolerance, Unit,
};
use tpt_av_cadence_core::{Encoder, FormatReader};
use tpt_av_cadence_wav::{WavEncoder, WavReader};
use tpt_dsp_analysis::{rms, zero_crossing_rate};
use tpt_dsp_audio::Oscillator;
use tpt_dsp_core::{fft, windowed, WindowType};

use crate::status::TestStatus;

/// Analysis stops counting clipping at this amplitude.
const CLIP_THRESHOLD: f32 = 0.999;
/// RMS below this level (dBFS) counts as silence.
const SILENCE_DBFS: f64 = -80.0;
/// Floor for the reported level of digital silence (keeps the measurement
/// finite; log10 of 0 is -inf).
const LEVEL_FLOOR_DBFS: f64 = -120.0;

/// A test tone to generate (§13.6). `level_dbfs` is the peak amplitude;
/// the RMS level of the generated sine sits ~3 dB below it.
#[derive(Debug, Clone, PartialEq)]
pub struct ToneSpec {
    pub frequency_hz: f64,
    pub level_dbfs: f64,
    pub duration_ms: u32,
    pub sample_rate_hz: u32,
}

impl Default for ToneSpec {
    fn default() -> Self {
        Self {
            frequency_hz: 1000.0,
            level_dbfs: -6.0,
            duration_ms: 100,
            sample_rate_hz: 48_000,
        }
    }
}

impl ToneSpec {
    /// The number of samples this tone spans.
    pub fn sample_count(&self) -> usize {
        (self.sample_rate_hz as u64 * self.duration_ms as u64 / 1000) as usize
    }
}

/// Generate the tone with `tpt-dsp-audio`'s oscillator.
pub fn generate_tone(spec: &ToneSpec) -> Vec<f32> {
    let mut oscillator = Oscillator::new(spec.sample_rate_hz as f32, spec.frequency_hz as f32);
    let mut raw = vec![0f32; spec.sample_count()];
    oscillator.process(&mut raw);
    let amplitude = 10f32.powf(spec.level_dbfs as f32 / 20.0);
    raw.iter().map(|s| s * amplitude).collect()
}

/// Errors from WAV I/O.
#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("codec error: {0}")]
    Codec(String),
}

/// Write mono samples as a 16-bit PCM WAV file.
pub fn write_wav(samples: &[f32], sample_rate_hz: u32, path: &Path) -> Result<(), AudioError> {
    let mut encoder = WavEncoder::new(
        File::create(path)?,
        sample_rate_hz,
        1,
        tpt_av_cadence_core::SampleFormat::Int16,
    )
    .map_err(|e| AudioError::Codec(e.to_string()))?;
    let mut written = 0usize;
    while written < samples.len() {
        let frames = encoder
            .encode(&samples[written..])
            .map_err(|e| AudioError::Codec(e.to_string()))?;
        if frames == 0 {
            return Err(AudioError::Codec("encoder stalled".to_owned()));
        }
        written += frames;
    }
    encoder
        .finish()
        .map_err(|e| AudioError::Codec(e.to_string()))?;
    Ok(())
}

/// Read a WAV file, downmixed to mono. Returns `(sample_rate, samples)`.
pub fn read_wav(path: &Path) -> Result<(u32, Vec<f32>), AudioError> {
    let mut reader = WavReader::open(Box::new(File::open(path)?))
        .map_err(|e| AudioError::Codec(e.to_string()))?;
    let (sample_rate, channels) = {
        let info = reader.info();
        (info.sample_rate, info.channels.max(1) as usize)
    };
    let decoder = reader.decoder();
    let mut samples = Vec::new();
    loop {
        let mut chunk = vec![0f32; channels * 1024];
        let frames = decoder
            .decode(&mut chunk)
            .map_err(|e| AudioError::Codec(e.to_string()))?;
        if frames == 0 {
            break;
        }
        for frame in chunk[..frames * channels].chunks(channels) {
            samples.push(frame.iter().sum::<f32>() / channels as f32);
        }
    }
    Ok((sample_rate, samples))
}

/// What [`analyze_tone`] should verify. Unset fields are not checked.
#[derive(Debug, Clone, Default)]
pub struct ToneExpectations {
    /// Expected dominant frequency with a tolerance.
    pub frequency_hz: Option<(f64, Tolerance)>,
    /// Expected RMS level range in dBFS, inclusive.
    pub level_dbfs: Option<(f64, f64)>,
    /// Maximum number of clipped samples (0 forbids clipping).
    pub max_clipped_samples: Option<usize>,
    /// The tone must not be digital silence.
    pub forbid_silence: bool,
}

/// The outcome of analysing one capture (§19 measurements).
#[derive(Debug, Clone, PartialEq)]
pub struct ToneAnalysis {
    pub status: TestStatus,
    pub measurements: Vec<Measurement>,
    pub messages: Vec<String>,
}

/// Analyse a captured buffer against the expectations.
///
/// Measurements (raw, unrounded): RMS level in dBFS, dominant frequency
/// (Hann-windowed FFT peak with parabolic interpolation), a zero-crossing
/// frequency estimate as a cross-check, and the clipped-sample count. With
/// no expectations set the status is `Inconclusive` — measurements without
/// an assertion never imply a pass.
pub fn analyze_tone(
    samples: &[f32],
    sample_rate_hz: u32,
    expectations: &ToneExpectations,
) -> ToneAnalysis {
    let mut measurements = Vec::new();
    let mut messages = Vec::new();
    let mut failures: Vec<String> = Vec::new();

    // Level: RMS in dBFS.
    let level_dbfs = level_dbfs_of(samples);
    let mut level = Measurement::new(
        "level_dbfs",
        MeasurementValue::Float(level_dbfs.unwrap_or(LEVEL_FLOOR_DBFS)),
    );
    level.unit = Some(Unit::DBFS);
    level.source = MeasurementSource::Computed;
    measurements.push(level);

    // Silence: digital silence fails a "no silence" expectation.
    let silent = level_dbfs.is_none_or(|db| db < SILENCE_DBFS);
    if silent {
        messages.push("capture is silent".to_owned());
        if expectations.forbid_silence {
            failures.push("capture is silent".to_owned());
        }
    }

    // Clipping.
    let clipped = samples.iter().filter(|s| s.abs() >= CLIP_THRESHOLD).count();
    let mut clip = Measurement::new("clipped_samples", MeasurementValue::Integer(clipped as i64));
    clip.source = MeasurementSource::Computed;
    measurements.push(clip);
    if let Some(max) = expectations.max_clipped_samples {
        if clipped > max {
            failures.push(format!(
                "clipped samples {clipped} exceed the maximum {max}"
            ));
        }
    }

    // Dominant frequency: Hann window + FFT peak. Needs at least a few
    // periods' worth of samples to be meaningful; shorter buffers skip it.
    if !silent && samples.len() >= 64 {
        if let Some(frequency_hz) = dominant_frequency(samples, sample_rate_hz) {
            let mut freq = Measurement::new("frequency_hz", MeasurementValue::Float(frequency_hz));
            freq.unit = Some(Unit::new("hz"));
            freq.source = MeasurementSource::Computed;
            measurements.push(freq);

            let zcr_hz = zero_crossing_rate(samples) as f64 * sample_rate_hz as f64 / 2.0;
            let mut zcr = Measurement::new("zcr_frequency_hz", MeasurementValue::Float(zcr_hz));
            zcr.unit = Some(Unit::new("hz"));
            zcr.source = MeasurementSource::Computed;
            measurements.push(zcr);

            if let Some((expected, tolerance)) = expectations.frequency_hz {
                let within = tolerance.check(
                    &MeasurementValue::Float(frequency_hz),
                    &MeasurementValue::Float(expected),
                );
                if within != Some(true) {
                    failures.push(format!(
                        "frequency {frequency_hz:.1} Hz is not within {expected:.1} Hz ({tolerance:?})"
                    ));
                }
            }
        }
    }

    // Level range expectation.
    if let Some((lo, hi)) = expectations.level_dbfs {
        match level_dbfs {
            None => failures.push("capture is silent; level cannot meet the range".to_owned()),
            Some(db) if db < lo || db > hi => {
                failures.push(format!("level {db:.1} dBFS outside [{lo}, {hi}] dBFS"))
            }
            _ => {}
        }
    }

    let status = if expectations.is_empty() {
        TestStatus::Inconclusive
    } else if failures.is_empty() {
        TestStatus::Pass
    } else {
        TestStatus::Fail
    };
    messages.extend(failures);
    ToneAnalysis {
        status,
        measurements,
        messages,
    }
}

impl ToneExpectations {
    fn is_empty(&self) -> bool {
        self.frequency_hz.is_none()
            && self.level_dbfs.is_none()
            && self.max_clipped_samples.is_none()
            && !self.forbid_silence
    }
}

/// RMS level in dBFS, or `None` for digital silence.
fn level_dbfs_of(samples: &[f32]) -> Option<f64> {
    let r = rms(samples) as f64;
    if r <= 0.0 {
        return None;
    }
    Some(20.0 * r.log10())
}

/// The dominant frequency of a windowed FFT, with parabolic peak
/// interpolation for sub-bin accuracy.
fn dominant_frequency(samples: &[f32], sample_rate_hz: u32) -> Option<f64> {
    // Largest power-of-two window that fits (capped to keep analysis cheap).
    let len = samples.len().min(8192);
    let mut size = 1usize;
    while size * 2 <= len {
        size *= 2;
    }
    if size < 16 {
        return None;
    }

    let mut window = vec![0f32; size];
    windowed(WindowType::Hann, size, &mut window);
    let input: Vec<Complex<f32>> = samples[..size]
        .iter()
        .zip(&window)
        .map(|(s, w)| Complex::new(s * w, 0.0))
        .collect();
    let mut out = vec![Complex::new(0.0, 0.0); size];
    let mut scratch = vec![Complex::new(0.0, 0.0); size];
    fft(&input, &mut out, &mut scratch);

    // Magnitudes of the positive-frequency half; skip DC.
    let magnitudes: Vec<f32> = out[1..size / 2].iter().map(|c| c.norm()).collect();
    let peak = magnitudes
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))?
        .0;
    let m = |i: usize| magnitudes.get(i).copied().unwrap_or(0.0);
    // Parabolic interpolation over the peak and its neighbours (in the
    // shifted index space, bin i corresponds to frequency (i+1)·rate/size).
    let (left, center, right) = (m(peak.wrapping_sub(1)), magnitudes[peak], m(peak + 1));
    let denominator = left - 2.0 * center + right;
    let delta = if denominator.abs() > f32::EPSILON {
        (-0.5 * (right - left) / denominator) as f64
    } else {
        0.0
    };
    let bin = (peak + 1) as f64 + delta.clamp(-1.0, 1.0);
    Some(bin * sample_rate_hz as f64 / size as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone_1khz() -> Vec<f32> {
        generate_tone(&ToneSpec::default())
    }

    #[test]
    fn generated_tone_matches_its_spec() {
        let samples = tone_1khz();
        assert_eq!(samples.len(), 4800); // 48 kHz · 100 ms
        let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        let expected_peak = 10f32.powf(-6.0 / 20.0);
        assert!((peak - expected_peak).abs() < 1e-3);
    }

    #[test]
    fn analysis_measures_frequency_and_level() {
        let samples = tone_1khz();
        let analysis = analyze_tone(
            &samples,
            48_000,
            &ToneExpectations {
                frequency_hz: Some((1000.0, Tolerance::Absolute { delta: 10.0 })),
                level_dbfs: Some((-10.5, -8.5)), // peak -6 dBFS ⇒ RMS ≈ -9 dBFS
                forbid_silence: true,
                ..ToneExpectations::default()
            },
        );
        assert_eq!(analysis.status, TestStatus::Pass, "{:?}", analysis.messages);
        let value = |name: &str| {
            analysis
                .measurements
                .iter()
                .find(|m| m.name == name)
                .map(|m| m.value.as_f64().unwrap())
        };
        let frequency = value("frequency_hz").unwrap();
        assert!((frequency - 1000.0).abs() < 10.0, "measured {frequency}");
        let zcr = value("zcr_frequency_hz").unwrap();
        assert!((zcr - 1000.0).abs() < 50.0, "zcr measured {zcr}");
        let level = value("level_dbfs").unwrap();
        assert!((-10.5..=-8.5).contains(&level), "level {level}");
    }

    #[test]
    fn wrong_frequency_fails() {
        let samples = generate_tone(&ToneSpec {
            frequency_hz: 440.0,
            ..ToneSpec::default()
        });
        let analysis = analyze_tone(
            &samples,
            48_000,
            &ToneExpectations {
                frequency_hz: Some((1000.0, Tolerance::Absolute { delta: 10.0 })),
                ..ToneExpectations::default()
            },
        );
        assert_eq!(analysis.status, TestStatus::Fail);
        assert!(analysis.messages.iter().any(|m| m.contains("frequency")));
    }

    #[test]
    fn silence_is_detected_and_can_fail() {
        let analysis = analyze_tone(
            &vec![0.0f32; 4800],
            48_000,
            &ToneExpectations {
                forbid_silence: true,
                ..ToneExpectations::default()
            },
        );
        assert_eq!(analysis.status, TestStatus::Fail);
        let level = analysis
            .measurements
            .iter()
            .find(|m| m.name == "level_dbfs")
            .unwrap()
            .value
            .as_f64()
            .unwrap();
        assert_eq!(level, LEVEL_FLOOR_DBFS);
    }

    #[test]
    fn clipping_is_counted_and_fails_when_forbidden() {
        let clipped: Vec<f32> = tone_1khz()
            .into_iter()
            .map(|s| s * 4.0)
            .map(|s| s.clamp(-1.0, 1.0))
            .collect();
        let analysis = analyze_tone(
            &clipped,
            48_000,
            &ToneExpectations {
                max_clipped_samples: Some(0),
                ..ToneExpectations::default()
            },
        );
        assert_eq!(analysis.status, TestStatus::Fail);
        let count = analysis
            .measurements
            .iter()
            .find(|m| m.name == "clipped_samples")
            .unwrap()
            .value
            .as_f64()
            .unwrap() as usize;
        assert!(count > 0);
    }

    #[test]
    fn no_expectations_is_inconclusive() {
        let analysis = analyze_tone(&tone_1khz(), 48_000, &ToneExpectations::default());
        assert_eq!(analysis.status, TestStatus::Inconclusive);
        assert!(!analysis.measurements.is_empty());
    }

    #[test]
    fn wav_round_trip_preserves_the_analysis() {
        let dir = std::env::temp_dir().join(format!("tpt-tone-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tone.wav");

        let samples = tone_1khz();
        write_wav(&samples, 48_000, &path).unwrap();
        let (rate, read_back) = read_wav(&path).unwrap();
        assert_eq!(rate, 48_000);
        assert_eq!(read_back.len(), samples.len());

        // 16-bit quantisation moves the level by a hair, not the verdict.
        let before = analyze_tone(
            &samples,
            48_000,
            &ToneExpectations {
                frequency_hz: Some((1000.0, Tolerance::Absolute { delta: 10.0 })),
                level_dbfs: Some((-11.0, -8.0)),
                ..ToneExpectations::default()
            },
        );
        let after = analyze_tone(
            &read_back,
            48_000,
            &ToneExpectations {
                frequency_hz: Some((1000.0, Tolerance::Absolute { delta: 10.0 })),
                level_dbfs: Some((-11.0, -8.0)),
                ..ToneExpectations::default()
            },
        );
        assert_eq!(before.status, TestStatus::Pass);
        assert_eq!(after.status, TestStatus::Pass);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn stereo_wav_is_downmixed() {
        let dir = std::env::temp_dir().join(format!("tpt-stereo-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("stereo.wav");
        let samples: Vec<f32> = tone_1khz()
            .iter()
            .flat_map(|s| [s * 0.25, s * 0.75])
            .collect();
        let mut encoder = WavEncoder::new(
            File::create(&path).unwrap(),
            48_000,
            2,
            tpt_av_cadence_core::SampleFormat::Int16,
        )
        .unwrap();
        encoder.encode(&samples).unwrap();
        encoder.finish().unwrap();

        let (rate, mono) = read_wav(&path).unwrap();
        assert_eq!(rate, 48_000);
        assert_eq!(mono.len(), samples.len() / 2);
        // (0.25 + 0.75) / 2 halves the amplitude: peak -6 dBFS - 6 dB,
        // RMS level ≈ -15 dBFS.
        let level = level_dbfs_of(&mono).unwrap();
        assert!((-16.0..=-14.0).contains(&level), "level {level}");

        std::fs::remove_dir_all(&dir).unwrap();
    }
}

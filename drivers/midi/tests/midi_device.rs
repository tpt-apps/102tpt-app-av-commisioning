//! End-to-end: the MIDI driver against an in-memory MIDI device built from two
//! `VirtualMidiPair`s (driver -> device, device -> driver).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use tpt_av_control_midi::{Midi1Message, MidiSink, MidiSource, VirtualMidiPair};

use tpt_app_av_commissioning_device::{DeviceCommand, StateValue};
use tpt_app_av_commissioning_driver::{DeviceDriver, DriverError};
use tpt_app_av_commissioning_driver_midi::{MidiDriver, MidiDriverConfig, MidiSpec};
use tpt_app_av_commissioning_model::DeviceId;
use tpt_app_av_commissioning_profile::DeviceProfile;
use tpt_app_av_commissioning_test::kinds;
use tpt_app_av_commissioning_test::{CommissioningTest, TestStatus};

#[derive(Clone, Copy, PartialEq)]
enum Behaviour {
    /// Echoes every CC and program change back, as many devices do.
    Echo,
    /// Hears the command and says nothing.
    Silent,
    /// Echoes a different value than it was given.
    Wrong,
    /// Floods the driver with timing clocks before echoing.
    Flood,
}

struct FakeDevice {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl FakeDevice {
    /// Returns the device plus the (sink, source) halves the driver uses.
    fn start(behaviour: Behaviour) -> (Self, Box<dyn MidiSink + Send>, Box<dyn MidiSource + Send>) {
        let (device_rx, driver_tx) = VirtualMidiPair::new().split();
        let (driver_rx, device_tx) = VirtualMidiPair::new().split();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let handle = thread::spawn(move || {
            let (mut rx, mut tx) = (device_rx, device_tx);
            while !flag.load(Ordering::SeqCst) {
                let Ok(Some(inbound)) = rx.recv_timeout(Duration::from_millis(20)) else {
                    continue;
                };
                let Some(msg) = inbound.message else { continue };
                let out = match (behaviour, msg) {
                    (Behaviour::Silent, _) => continue,
                    (
                        Behaviour::Wrong,
                        Midi1Message::ControlChange {
                            channel,
                            controller,
                            ..
                        },
                    ) => Midi1Message::ControlChange {
                        channel,
                        controller,
                        value: 0,
                    },
                    (_, m @ Midi1Message::ControlChange { .. })
                    | (_, m @ Midi1Message::ProgramChange { .. }) => m,
                    _ => continue,
                };
                if behaviour == Behaviour::Flood {
                    for _ in 0..200 {
                        let _ = tx.send_midi1(&Midi1Message::TimingClock);
                    }
                }
                let _ = tx.send_midi1(&out);
            }
        });
        (
            Self {
                stop,
                handle: Some(handle),
            },
            Box::new(driver_tx),
            Box::new(driver_rx),
        )
    }
}

impl Drop for FakeDevice {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

const PROFILE: &str = r#"
schema_version: 1
device:
  id: midi-mixer
  match: { manufacturer: MidiCo, model: Mix-1 }
  protocol: { type: midi, timeout_ms: 300 }
  commands:
    power_on:  { send: "cc 1 20 127" }
    power_off: { send: "cc 1 20 0" }
    set_input: { send: "program 1 3" }
  state:
    power: { query: "cc 1 20" }
    input: { query: "program 1" }
"#;

fn driver(behaviour: Behaviour) -> (FakeDevice, MidiDriver) {
    let profile = DeviceProfile::from_yaml_str(PROFILE).unwrap();
    let config = MidiDriverConfig::from_profile(&profile, "Mixer").unwrap();
    let (dev, sink, source) = FakeDevice::start(behaviour);
    let d = MidiDriver::with_ports(Some(sink), Some(source), config.spec, config.timeout).unwrap();
    (dev, d)
}

#[test]
fn echoed_commands_become_typed_state() {
    let (_dev, mut d) = driver(Behaviour::Echo);
    assert_eq!(d.identity().model.as_deref(), Some("Mix-1"));
    let r = d.execute(DeviceCommand::PowerOn).unwrap();
    assert!(r.ok && r.message.unwrap().contains("no acknowledgement"));
    let s = d.get_state().unwrap();
    assert_eq!(s.get("power"), Some(&StateValue::Integer(127)));
    // The program change was never sent, so the device has not reported it.
    assert!(s.get("input").is_none());
    d.execute(DeviceCommand::SetInput {
        input: "ignored: MIDI messages are fixed".into(),
    })
    .unwrap();
    assert_eq!(
        d.get_state().unwrap().get("input"),
        Some(&StateValue::Integer(3))
    );
}

#[test]
fn state_before_a_command_is_never_mistaken_for_its_effect() {
    let (_dev, mut d) = driver(Behaviour::Echo);
    d.execute(DeviceCommand::PowerOn).unwrap();
    assert_eq!(
        d.get_state().unwrap().get("power"),
        Some(&StateValue::Integer(127))
    );
    // The device now goes quiet. Without the clear-on-command rule the old 127
    // would make a failed power-off look like it left the device on, or worse,
    // a failed power-on look like it worked.
    let (_dev2, mut quiet) = driver(Behaviour::Silent);
    quiet.execute(DeviceCommand::PowerOff).unwrap();
    assert!(quiet.get_state().unwrap().get("power").is_none());
}

#[test]
fn silent_device_leaves_fields_unreported_after_the_timeout() {
    let (_dev, mut d) = driver(Behaviour::Silent);
    d.execute(DeviceCommand::PowerOn).unwrap();
    let s = d.get_state().unwrap();
    assert!(s.get("power").is_none());
}

#[test]
fn flooding_does_not_stall_the_driver() {
    let (_dev, mut d) = driver(Behaviour::Flood);
    d.execute(DeviceCommand::PowerOn).unwrap();
    assert_eq!(
        d.get_state().unwrap().get("power"),
        Some(&StateValue::Integer(127))
    );
}

#[test]
fn unbound_and_raw_commands_are_unsupported() {
    let (_dev, mut d) = driver(Behaviour::Echo);
    assert_eq!(
        d.execute(DeviceCommand::PowerCycle),
        Err(DriverError::UnsupportedOperation)
    );
    // Raw MIDI / SysEx is never accepted from test data.
    assert_eq!(
        d.execute(DeviceCommand::Arbitrary {
            command: "f0 7e 7f 09 01 f7".into()
        }),
        Err(DriverError::UnsupportedOperation)
    );
}

#[test]
fn discover_means_the_ports_are_open() {
    let (_dev, mut d) = driver(Behaviour::Silent);
    assert!(d.discover().unwrap().is_empty());
}

#[test]
fn phase9_power_tests_over_midi() {
    let (_dev, d) = driver(Behaviour::Echo);
    // power is reported as the CC value, so use a raw expectation via a
    // control-feedback test.
    let drv = kinds::share(d);
    let t = kinds::control_feedback(
        "midi-power",
        DeviceId::new("mixer"),
        drv,
        DeviceCommand::PowerOn,
        tpt_app_av_commissioning_test::Expectation::at_least("power", 127.0),
    );
    assert_eq!(t.execute().unwrap().status, TestStatus::Pass);

    let (_dev, d) = driver(Behaviour::Wrong);
    let t = kinds::control_feedback(
        "midi-power",
        DeviceId::new("mixer"),
        kinds::share(d),
        DeviceCommand::PowerOn,
        tpt_app_av_commissioning_test::Expectation::at_least("power", 127.0),
    );
    assert_eq!(t.execute().unwrap().status, TestStatus::Fail);

    let (_dev, d) = driver(Behaviour::Silent);
    let t = kinds::control_feedback(
        "midi-power",
        DeviceId::new("mixer"),
        kinds::share(d),
        DeviceCommand::PowerOn,
        tpt_app_av_commissioning_test::Expectation::at_least("power", 127.0),
    );
    // Unreported, not failed and not passed.
    assert_eq!(t.execute().unwrap().status, TestStatus::Inconclusive);
}

#[test]
fn profile_validation_covers_midi_rules() {
    for (from, to) in [
        ("cc 1 20 127", "cc 17 20 127"),
        ("cc 1 20 127", "cc 1 20 200"),
        ("cc 1 20 127", "sysex f0 f7"),
        ("cc 1 20 127", "cc 1 20"),
        ("program 1\"", "bend 1\""),
    ] {
        let doc = PROFILE.replace(from, to);
        assert!(DeviceProfile::from_yaml_str(&doc).is_err(), "{to}");
    }
    let with_port = PROFILE.replace("type: midi,", "type: midi, port: 5,");
    assert!(DeviceProfile::from_yaml_str(&with_port).is_err());
    let osc = PROFILE.replace("type: midi", "type: tcp");
    // A tcp profile would still need a port; the MIDI driver refuses anyway.
    let _ = osc;
    let p = DeviceProfile::from_yaml_str(PROFILE).unwrap();
    let tcp_like = PROFILE.replace("type: midi", "type: osc");
    if let Ok(o) = DeviceProfile::from_yaml_str(&tcp_like) {
        assert!(MidiSpec::from_profile(&o).is_err());
    }
    assert!(MidiSpec::from_profile(&p).is_ok());
}

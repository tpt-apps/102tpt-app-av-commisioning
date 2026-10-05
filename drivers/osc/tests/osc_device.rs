//! End-to-end: the OSC driver against a fake OSC device on loopback, driven by
//! the Phase 9 commissioning tests.

use std::collections::HashMap;
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use tpt_app_av_commissioning_device::{DeviceCommand, DeviceIdentity, StateValue};
use tpt_app_av_commissioning_driver::{DeviceDriver, DriverError};
use tpt_app_av_commissioning_driver_osc::{
    Binding, BindingArg, OscDriver, OscDriverConfig, StateQuery,
};
use tpt_app_av_commissioning_model::DeviceId;
use tpt_app_av_commissioning_test::kinds::{self, ConnectivityProbe, ConnectivityTest};
use tpt_app_av_commissioning_test::{CommissioningTest, TestStatus};
use tpt_av_control_osc::{OscArg, OscMessage};

/// A projector-ish OSC device: a message with arguments sets that address; a
/// message without arguments queries it and is answered from the same socket.
struct FakeDevice {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

#[derive(Clone, Copy, PartialEq)]
enum Behaviour {
    Normal,
    /// Acknowledge nothing and never change state.
    IgnoreSets,
    /// Reply with garbage.
    Garbage,
}

impl FakeDevice {
    fn start(initial: &[(&str, OscArg)], behaviour: Behaviour) -> Self {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket
            .set_read_timeout(Some(Duration::from_millis(20)))
            .unwrap();
        let addr = socket.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let mut state: HashMap<String, OscArg> = initial
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .collect();
        let handle = thread::spawn(move || {
            let mut buf = vec![0u8; 65_507];
            while !flag.load(Ordering::SeqCst) {
                let Ok((len, from)) = socket.recv_from(&mut buf) else {
                    continue;
                };
                let Ok(msg) = OscMessage::decode(&buf[..len]) else {
                    continue;
                };
                if behaviour == Behaviour::Garbage {
                    let _ = socket.send_to(&[0xff, 0xfe, 0x00], from);
                    continue;
                }
                if msg.arguments.is_empty() {
                    if let Some(v) = state.get(&msg.address) {
                        let reply = OscMessage::new_unchecked(msg.address, vec![v.clone()]);
                        let _ = socket.send_to(&reply.encode(), from);
                    }
                } else if behaviour == Behaviour::Normal {
                    state.insert(msg.address, msg.arguments[0].clone());
                }
            }
        });
        Self {
            addr,
            stop,
            handle: Some(handle),
        }
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

fn config(target: SocketAddr) -> OscDriverConfig {
    OscDriverConfig::new(target)
        .timeout(Duration::from_millis(400))
        .identity(DeviceIdentity {
            manufacturer: Some("OscCo".into()),
            model: Some("Beam-1".into()),
            serial_number: Some("S1".into()),
            firmware: Some("1.0".into()),
        })
        .bind(
            "power_on",
            Binding::new("/power", vec![BindingArg::Bool(true)]),
        )
        .bind(
            "power_off",
            Binding::new("/power", vec![BindingArg::Bool(false)]),
        )
        .bind("set_input", Binding::new("/input", vec![BindingArg::Input]))
        .query(StateQuery::new("power", "/power"))
        .query(StateQuery::new("input", "/input"))
        .query(StateQuery::new("signal_present", "/signal"))
}

fn initial() -> Vec<(&'static str, OscArg)> {
    vec![
        ("/power", OscArg::Bool(false)),
        ("/input", OscArg::String("hdmi1".into())),
        ("/signal", OscArg::Bool(true)),
    ]
}

fn driver(dev: &FakeDevice) -> OscDriver {
    OscDriver::new(config(dev.addr)).unwrap()
}

fn id() -> DeviceId {
    DeviceId::new("proj-01")
}

#[test]
fn reads_typed_state_from_queries() {
    let dev = FakeDevice::start(&initial(), Behaviour::Normal);
    let mut d = driver(&dev);
    let s = d.get_state().unwrap();
    assert_eq!(s.get("power"), Some(&StateValue::Boolean(false)));
    assert_eq!(s.get("input"), Some(&StateValue::Text("hdmi1".into())));
    assert!(d.capabilities().can_power_on && d.capabilities().can_read_signal_status);
    assert_eq!(d.identity().model.as_deref(), Some("Beam-1"));
}

#[test]
fn command_is_fire_and_forget_and_state_confirms_it() {
    let dev = FakeDevice::start(&initial(), Behaviour::Normal);
    let mut d = driver(&dev);
    let r = d.execute(DeviceCommand::PowerOn).unwrap();
    assert!(r.ok && r.message.unwrap().contains("no acknowledgement"));
    assert_eq!(
        d.get_state().unwrap().get("power"),
        Some(&StateValue::Boolean(true))
    );
}

#[test]
fn unbound_command_is_unsupported() {
    let dev = FakeDevice::start(&initial(), Behaviour::Normal);
    let mut d = driver(&dev);
    assert_eq!(
        d.execute(DeviceCommand::PowerCycle),
        Err(DriverError::UnsupportedOperation)
    );
}

#[test]
fn arbitrary_command_reaches_the_device() {
    let dev = FakeDevice::start(&initial(), Behaviour::Normal);
    let mut d =
        OscDriver::new(config(dev.addr).query(StateQuery::new("preset", "/preset"))).unwrap();
    d.execute(DeviceCommand::Arbitrary {
        command: "/preset 4".into(),
    })
    .unwrap();
    assert_eq!(
        d.get_state().unwrap().get("preset"),
        Some(&StateValue::Integer(4))
    );
}

#[test]
fn silent_device_times_out() {
    // Bound but never answers.
    let silent = UdpSocket::bind("127.0.0.1:0").unwrap();
    let mut d = driver_for(silent.local_addr().unwrap());
    assert!(matches!(d.get_state(), Err(DriverError::Timeout(_))));
}

#[test]
fn closed_port_is_unreachable_or_times_out() {
    let addr = {
        let s = UdpSocket::bind("127.0.0.1:0").unwrap();
        s.local_addr().unwrap()
    };
    let mut d = driver_for(addr);
    assert!(matches!(
        d.get_state(),
        Err(DriverError::Unreachable(_)) | Err(DriverError::Timeout(_))
    ));
}

#[test]
fn garbage_reply_is_a_malformed_response_not_a_crash() {
    let dev = FakeDevice::start(&initial(), Behaviour::Garbage);
    let mut d = driver(&dev);
    assert!(matches!(
        d.get_state(),
        Err(DriverError::MalformedResponse(_))
    ));
}

fn driver_for(addr: SocketAddr) -> OscDriver {
    OscDriver::new(config(addr)).unwrap()
}

#[test]
fn discover_requires_state_queries() {
    let dev = FakeDevice::start(&initial(), Behaviour::Normal);
    let mut d = OscDriver::new(OscDriverConfig::new(dev.addr)).unwrap();
    assert!(matches!(d.discover(), Err(DriverError::Config(_))));
}

// --- the Phase 9 tests, running over real OSC -------------------------------

#[test]
fn phase9_connectivity_and_identity_tests_over_osc() {
    let dev = FakeDevice::start(&initial(), Behaviour::Normal);
    let drv = kinds::share(driver(&dev));
    let t = ConnectivityTest::new("osc-conn", id(), ConnectivityProbe::Osc, drv.clone());
    assert_eq!(t.execute().unwrap().status, TestStatus::Pass);

    let expected = DeviceIdentity {
        model: Some("Beam-1".into()),
        manufacturer: Some("OscCo".into()),
        ..DeviceIdentity::default()
    };
    let t = kinds::identity("osc-id", id(), drv, &expected, None);
    assert_eq!(t.execute().unwrap().status, TestStatus::Pass);
}

#[test]
fn phase9_power_on_test_passes_against_a_responsive_device() {
    let dev = FakeDevice::start(&initial(), Behaviour::Normal);
    let t = kinds::power_on("osc-pwr", id(), kinds::share(driver(&dev)));
    assert_eq!(t.execute().unwrap().status, TestStatus::Pass);
}

#[test]
fn phase9_power_on_test_fails_when_device_ignores_the_command() {
    // The command is sent fine (OSC cannot tell), but state never changes:
    // the independent read-back is what catches it.
    let dev = FakeDevice::start(&initial(), Behaviour::IgnoreSets);
    let t = kinds::power_on("osc-pwr", id(), kinds::share(driver(&dev)));
    assert_eq!(t.execute().unwrap().status, TestStatus::Fail);
}

#[test]
fn phase9_select_input_test() {
    let dev = FakeDevice::start(&initial(), Behaviour::Normal);
    let t = kinds::select_input("osc-in", id(), kinds::share(driver(&dev)), "hdmi2");
    assert_eq!(t.execute().unwrap().status, TestStatus::Pass);
}

#[test]
fn phase9_unreachable_osc_device_fails_connectivity() {
    let silent = UdpSocket::bind("127.0.0.1:0").unwrap();
    let drv = kinds::share(driver_for(silent.local_addr().unwrap()));
    let t = ConnectivityTest::new("osc-conn", id(), ConnectivityProbe::Osc, drv);
    assert_eq!(t.execute().unwrap().status, TestStatus::Fail);
}

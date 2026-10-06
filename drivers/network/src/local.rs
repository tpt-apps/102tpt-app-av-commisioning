//! Enumeration of this machine's own ports and devices: MIDI, serial, audio.
//!
//! Nothing here touches the network. Names come from the operating system but
//! are still passed through [`sanitize`] before being stored, since a USB
//! device chooses its own strings.

use tpt_app_av_commissioning_device::DeviceIdentity;
use tpt_app_av_commissioning_driver::{DiscoveredDevice, DriverError};

use crate::util::sanitize;

pub(crate) fn midi() -> Result<Vec<DiscoveredDevice>, DriverError> {
    let devices = tpt_av_control_midi::enumerate_devices()
        .map_err(|e| DriverError::Protocol(format!("MIDI enumeration failed: {e}")))?;
    Ok(devices
        .into_iter()
        .map(|d| {
            let name = sanitize(&d.name, 128);
            DiscoveredDevice::new(format!("midi:{name}"), "midi")
                .detail("name", name)
                .detail("inputs", d.inputs.len().to_string())
                .detail("outputs", d.outputs.len().to_string())
        })
        .collect())
}

pub(crate) fn serial() -> Result<Vec<DiscoveredDevice>, DriverError> {
    let ports = serialport::available_ports()
        .map_err(|e| DriverError::Protocol(format!("serial enumeration failed: {e}")))?;
    Ok(ports
        .into_iter()
        .map(|p| {
            let mut device = DiscoveredDevice::new(sanitize(&p.port_name, 128), "serial");
            match p.port_type {
                serialport::SerialPortType::UsbPort(usb) => {
                    device = device
                        .detail("type", "usb")
                        .detail("vid_pid", format!("{:04x}:{:04x}", usb.vid, usb.pid));
                    // A USB serial adapter names its maker and product: that
                    // is identity a profile can match.
                    device.identity = DeviceIdentity {
                        manufacturer: usb.manufacturer.map(|m| sanitize(&m, 128)),
                        model: usb.product.map(|m| sanitize(&m, 128)),
                        serial_number: usb.serial_number.map(|m| sanitize(&m, 128)),
                        firmware: None,
                    };
                }
                serialport::SerialPortType::BluetoothPort => {
                    device = device.detail("type", "bluetooth");
                }
                serialport::SerialPortType::PciPort => {
                    device = device.detail("type", "pci");
                }
                serialport::SerialPortType::Unknown => {
                    device = device.detail("type", "unknown");
                }
            }
            device
        })
        .collect())
}

pub(crate) fn audio() -> Result<Vec<DiscoveredDevice>, DriverError> {
    let devices = tpt_av_audio_io::enumerate_devices()
        .map_err(|e| DriverError::Protocol(format!("audio enumeration failed: {e}")))?;
    Ok(devices
        .into_iter()
        .map(|d| {
            let name = sanitize(&d.name, 128);
            let direction = match d.direction {
                tpt_av_audio_io::Direction::Input => "input",
                tpt_av_audio_io::Direction::Output => "output",
            };
            DiscoveredDevice::new(format!("audio:{}", sanitize(&d.id.0, 128)), "audio")
                .detail("name", name)
                .detail("direction", direction)
                .detail("input_channels", d.input_channels.to_string())
                .detail("output_channels", d.output_channels.to_string())
                .detail("default", d.is_default.to_string())
        })
        .collect())
}

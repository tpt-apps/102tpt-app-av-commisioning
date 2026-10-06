//! Serial (RS-232/RS-422/RS-485 via USB or native ports) text-protocol driver.
//!
//! Same command/query model as TCP ([`crate::text`]) over a serial line. The
//! port is opened per operation and closed afterwards.
//!
//! Safety: only genuine serial device paths are accepted (`COMn` / `\\.\COMn`
//! on Windows, `/dev/...` elsewhere), so a profile or project file cannot make
//! the tool open an arbitrary file. Every read is bounded by the timeout.
//!
//! Not covered by automated tests that need hardware: opening a real port.
//! Framing, parsing and safety logic are shared with the TCP driver and tested
//! there; this module's own tests cover configuration and error handling.

use std::io::{Read, Write};
use std::time::Duration;

use tpt_app_av_commissioning_device::DeviceIdentity;
use tpt_app_av_commissioning_driver::net::{check_timeout, map_io_error};
use tpt_app_av_commissioning_driver::DriverError;
use tpt_app_av_commissioning_profile::{
    DeviceProfile, Parity as ProfileParity, ProtocolKind, SerialSpec, Terminator,
};

use crate::text::{
    text_config_builders, Exchange, Opener, StreamExchange, TextCommand, TextDriver, TextQuery,
    TextSpec, Transport,
};

/// Parity setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Parity {
    None,
    Even,
    Odd,
}

impl From<ProfileParity> for Parity {
    fn from(p: ProfileParity) -> Self {
        match p {
            ProfileParity::None => Parity::None,
            ProfileParity::Even => Parity::Even,
            ProfileParity::Odd => Parity::Odd,
        }
    }
}

/// Line settings. The default is 9600 8N1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineSettings {
    pub baud: u32,
    pub data_bits: u8,
    pub parity: Parity,
    pub stop_bits: u8,
}

impl Default for LineSettings {
    fn default() -> Self {
        Self {
            baud: 9600,
            data_bits: 8,
            parity: Parity::None,
            stop_bits: 1,
        }
    }
}

impl From<&SerialSpec> for LineSettings {
    fn from(s: &SerialSpec) -> Self {
        Self {
            baud: s.baud,
            data_bits: s.data_bits.unwrap_or(8),
            parity: s.parity.map(Parity::from).unwrap_or(Parity::None),
            stop_bits: s.stop_bits.unwrap_or(1),
        }
    }
}

/// Everything the serial driver needs to know about one device.
#[derive(Debug, Clone)]
pub struct SerialDriverConfig {
    /// `COM3`, `/dev/ttyUSB0`, ...
    pub path: String,
    pub line: LineSettings,
    pub timeout: Duration,
    pub terminator: Terminator,
    pub spec: TextSpec,
}

impl SerialDriverConfig {
    /// A config for the port at `path`: 9600 8N1, 1 s timeout, CR terminator.
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            line: LineSettings::default(),
            timeout: Duration::from_secs(1),
            terminator: Terminator::Cr,
            spec: TextSpec::default(),
        }
    }

    pub fn line(mut self, line: LineSettings) -> Self {
        self.line = line;
        self
    }

    text_config_builders!();

    /// Build a config from a `serial` device profile, for the port at `path`.
    pub fn from_profile(profile: &DeviceProfile, path: &str) -> Result<Self, DriverError> {
        let p = &profile.device;
        if p.protocol.kind != ProtocolKind::Serial {
            return Err(DriverError::Config(format!(
                "profile `{}` is a {:?} profile; the serial driver needs `type: serial`",
                p.id, p.protocol.kind
            )));
        }
        let serial = p.protocol.serial.as_ref().ok_or_else(|| {
            DriverError::Config(format!("profile `{}` has no `protocol.serial`", p.id))
        })?;
        let config = Self {
            path: path.to_owned(),
            line: LineSettings::from(serial),
            timeout: Duration::from_millis(p.protocol.timeout_ms_or_default()),
            terminator: p.protocol.terminator_or_default(),
            spec: TextSpec::from_profile(profile)?,
        };
        config.validate()?;
        Ok(config)
    }

    /// Check the config is safe and coherent. [`SerialDriver::new`] calls this.
    pub fn validate(&self) -> Result<(), DriverError> {
        check_serial_path(&self.path)?;
        check_timeout(self.timeout)?;
        if !(300..=4_000_000).contains(&self.line.baud)
            || !(5..=8).contains(&self.line.data_bits)
            || !matches!(self.line.stop_bits, 1 | 2)
        {
            return Err(DriverError::Config(
                "unsupported serial line settings".to_owned(),
            ));
        }
        self.spec.validate()
    }
}

/// Accept only real serial device paths.
pub fn check_serial_path(path: &str) -> Result<(), DriverError> {
    let bad = || {
        DriverError::Config(format!(
            "{path:?} is not a serial port path (expected COMn, \\\\.\\COMn or /dev/...)"
        ))
    };
    if path.len() > 256 || path.chars().any(char::is_control) || path.contains("..") {
        return Err(bad());
    }
    let upper = path.to_ascii_uppercase();
    let com = upper
        .strip_prefix("\\\\.\\")
        .unwrap_or(&upper)
        .strip_prefix("COM");
    let is_com = com.is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
    let is_dev = path.starts_with("/dev/") && path.len() > "/dev/".len();
    if is_com || is_dev {
        Ok(())
    } else {
        Err(bad())
    }
}

/// An open serial port as a [`Transport`].
struct SerialStream(Box<dyn serialport::SerialPort>);

impl Read for SerialStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl Write for SerialStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

impl Transport for SerialStream {
    fn set_read_timeout(&mut self, timeout: Duration) -> std::io::Result<()> {
        self.0.set_timeout(timeout).map_err(std::io::Error::from)
    }
}

/// Opens the serial port per operation.
pub struct SerialOpener {
    path: String,
    line: LineSettings,
    timeout: Duration,
    terminator: Terminator,
}

fn map_serial_error(e: serialport::Error, path: &str, timeout: Duration) -> DriverError {
    match e.kind {
        serialport::ErrorKind::NoDevice => {
            DriverError::Unreachable(format!("{path}: no such serial port"))
        }
        serialport::ErrorKind::InvalidInput => {
            DriverError::Config(format!("{path}: {}", e.description))
        }
        serialport::ErrorKind::Io(kind) => {
            map_io_error(&std::io::Error::new(kind, e.description), path, timeout)
        }
        serialport::ErrorKind::Unknown => {
            DriverError::Protocol(format!("{path}: {}", e.description))
        }
    }
}

impl Opener for SerialOpener {
    fn open(&self) -> Result<Box<dyn Exchange + '_>, DriverError> {
        let data_bits = match self.line.data_bits {
            5 => serialport::DataBits::Five,
            6 => serialport::DataBits::Six,
            7 => serialport::DataBits::Seven,
            _ => serialport::DataBits::Eight,
        };
        let parity = match self.line.parity {
            Parity::None => serialport::Parity::None,
            Parity::Even => serialport::Parity::Even,
            Parity::Odd => serialport::Parity::Odd,
        };
        let stop_bits = if self.line.stop_bits == 2 {
            serialport::StopBits::Two
        } else {
            serialport::StopBits::One
        };
        let port = serialport::new(&self.path, self.line.baud)
            .data_bits(data_bits)
            .parity(parity)
            .stop_bits(stop_bits)
            .timeout(self.timeout)
            .open()
            .map_err(|e| map_serial_error(e, &self.path, self.timeout))?;
        Ok(Box::new(StreamExchange::new(
            SerialStream(port),
            self.terminator,
            self.timeout,
            self.path.clone(),
        )))
    }
}

/// A text-protocol device on a serial line.
pub type SerialDriver = TextDriver<SerialOpener>;

impl TextDriver<SerialOpener> {
    /// Validate `config`. The port is not opened until an operation runs.
    pub fn new(config: SerialDriverConfig) -> Result<Self, DriverError> {
        config.validate()?;
        TextDriver::with_opener(
            SerialOpener {
                path: config.path,
                line: config.line,
                timeout: config.timeout,
                terminator: config.terminator,
            },
            config.spec,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tpt_app_av_commissioning_driver::DeviceDriver;

    #[test]
    fn only_serial_paths_are_accepted() {
        for ok in [
            "COM3",
            "com12",
            "\\\\.\\COM10",
            "/dev/ttyUSB0",
            "/dev/cu.usbserial-1410",
        ] {
            assert!(check_serial_path(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "COM",
            "COMx",
            "/etc/passwd",
            "/dev/",
            "/dev/../etc/passwd",
            "C:\\Windows\\system.ini",
            "relative/file",
            "COM1\n",
            "\\\\server\\share",
        ] {
            assert!(check_serial_path(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn validates_line_settings() {
        let ok = SerialDriverConfig::new("COM3");
        assert!(ok.validate().is_ok());
        for line in [
            LineSettings {
                baud: 10,
                ..LineSettings::default()
            },
            LineSettings {
                data_bits: 9,
                ..LineSettings::default()
            },
            LineSettings {
                stop_bits: 3,
                ..LineSettings::default()
            },
        ] {
            assert!(ok.clone().line(line).validate().is_err());
        }
        assert!(ok.clone().timeout(Duration::ZERO).validate().is_err());
        assert!(ok
            .bind("power_on", TextCommand::new("a\nb"))
            .validate()
            .is_err());
    }

    #[test]
    fn missing_port_is_an_error_not_a_panic() {
        let path = if cfg!(windows) {
            "COM250"
        } else {
            "/dev/tpt-no-such-port"
        };
        let mut d = SerialDriver::new(
            SerialDriverConfig::new(path).query(TextQuery::new("power", "POWR?")),
        )
        .unwrap();
        assert!(matches!(
            d.get_state(),
            Err(DriverError::Unreachable(_)) | Err(DriverError::Protocol(_))
        ));
    }

    #[test]
    fn identity_is_carried_through() {
        let c = SerialDriverConfig::new("COM1").identity(DeviceIdentity {
            model: Some("M".into()),
            ..DeviceIdentity::default()
        });
        assert_eq!(c.spec.identity.model.as_deref(), Some("M"));
        let _ = TextQuery::new("a", "b");
    }
}

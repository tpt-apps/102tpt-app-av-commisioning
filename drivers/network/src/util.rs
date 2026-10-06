//! Shared helpers: interface selection, bounded parallelism, text hygiene.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use tpt_app_av_commissioning_driver::DriverError;

use crate::CancelFlag;

/// What one mechanism needs to run.
pub(crate) struct Ctx<'a> {
    pub hosts: &'a [IpAddr],
    /// Local IPv4 address to bind to, if the caller selected an interface.
    pub interface: Option<Ipv4Addr>,
    pub timeout: Duration,
    pub concurrency: usize,
    pub cancel: &'a CancelFlag,
}

/// A local network interface address, for the interface picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterfaceInfo {
    pub name: String,
    pub address: Ipv4Addr,
    pub is_loopback: bool,
}

/// The machine's IPv4 interface addresses.
pub fn list_interfaces() -> Result<Vec<InterfaceInfo>, DriverError> {
    let all = if_addrs::get_if_addrs()
        .map_err(|e| DriverError::Protocol(format!("could not list interfaces: {e}")))?;
    let mut out: Vec<InterfaceInfo> = all
        .into_iter()
        .filter_map(|i| match i.ip() {
            IpAddr::V4(v4) => Some(InterfaceInfo {
                is_loopback: i.is_loopback(),
                name: i.name,
                address: v4,
            }),
            IpAddr::V6(_) => None,
        })
        .collect();
    out.sort_by(|a, b| (&a.name, a.address).cmp(&(&b.name, b.address)));
    Ok(out)
}

/// Resolve an interface selection (an IPv4 address or an interface name) to
/// one of this machine's addresses. An address that is not on this machine is
/// refused: binding to it would fail confusingly later.
pub fn resolve_interface(selection: &str) -> Result<Ipv4Addr, DriverError> {
    let selection = selection.trim();
    let interfaces = list_interfaces()?;
    if let Ok(ip) = selection.parse::<Ipv4Addr>() {
        return interfaces
            .iter()
            .find(|i| i.address == ip)
            .map(|i| i.address)
            .ok_or_else(|| DriverError::Config(format!("{ip} is not an address on this machine")));
    }
    let matches: Vec<&InterfaceInfo> = interfaces
        .iter()
        .filter(|i| i.name.eq_ignore_ascii_case(selection))
        .collect();
    match matches.as_slice() {
        [] => Err(DriverError::Config(format!(
            "no interface named {selection:?} (use list_interfaces to see them)"
        ))),
        [one] => Ok(one.address),
        // An interface with several IPv4 addresses is ambiguous: make the
        // caller pick one rather than guess.
        many => Err(DriverError::Config(format!(
            "interface {selection:?} has {} IPv4 addresses; select one by address",
            many.len()
        ))),
    }
}

/// Run `f` over `items` on up to `concurrency` threads, stopping early when
/// `cancel` is set. Results come back in item order, `None`s dropped.
pub(crate) fn parallel_map<T: Sync, R: Send>(
    items: &[T],
    concurrency: usize,
    cancel: &CancelFlag,
    f: impl Fn(&T) -> Option<R> + Sync,
) -> Vec<R> {
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<(usize, R)>> = Mutex::new(Vec::new());
    let workers = concurrency.clamp(1, items.len().max(1));
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                if cancel.is_cancelled() {
                    return;
                }
                let i = next.fetch_add(1, Ordering::SeqCst);
                let Some(item) = items.get(i) else { return };
                if let Some(r) = f(item) {
                    results.lock().unwrap().push((i, r));
                }
            });
        }
    });
    let mut results = results.into_inner().unwrap();
    results.sort_by_key(|(i, _)| *i);
    results.into_iter().map(|(_, r)| r).collect()
}

/// Make device-supplied text safe to show and store: printable ASCII only
/// (anything else becomes `.`), trimmed, and at most `max` characters.
pub(crate) fn sanitize(text: &str, max: usize) -> String {
    text.trim()
        .chars()
        .map(|c| {
            if c.is_ascii_graphic() || c == ' ' {
                c
            } else {
                '.'
            }
        })
        .take(max)
        .collect::<String>()
        .trim()
        .to_owned()
}

/// Connect with a timeout, optionally from a chosen local IPv4 address.
pub(crate) fn connect(
    target: SocketAddr,
    timeout: Duration,
    bind: Option<Ipv4Addr>,
) -> std::io::Result<TcpStream> {
    match (bind, target) {
        (Some(local), SocketAddr::V4(_)) => {
            let socket = socket2::Socket::new(
                socket2::Domain::IPV4,
                socket2::Type::STREAM,
                Some(socket2::Protocol::TCP),
            )?;
            socket.bind(&SocketAddr::V4(SocketAddrV4::new(local, 0)).into())?;
            socket.connect_timeout(&target.into(), timeout)?;
            Ok(socket.into())
        }
        _ => TcpStream::connect_timeout(&target, timeout),
    }
}

/// A UDP socket bound to the chosen interface (or any), connected to
/// `target`.
pub(crate) fn udp_to(target: SocketAddr, bind: Option<Ipv4Addr>) -> std::io::Result<UdpSocket> {
    let local: SocketAddr = match (bind, target) {
        (Some(ip), SocketAddr::V4(_)) => SocketAddr::V4(SocketAddrV4::new(ip, 0)),
        (_, SocketAddr::V4(_)) => ([0, 0, 0, 0], 0).into(),
        (_, SocketAddr::V6(_)) => (std::net::Ipv6Addr::UNSPECIFIED, 0).into(),
    };
    let socket = UdpSocket::bind(local)?;
    socket.connect(target)?;
    Ok(socket)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_interface_resolves_by_address_and_foreign_addresses_are_refused() {
        let lo = list_interfaces()
            .unwrap()
            .into_iter()
            .find(|i| i.is_loopback)
            .expect("a loopback interface");
        assert_eq!(
            resolve_interface(&lo.address.to_string()).unwrap(),
            lo.address
        );
        assert_eq!(
            resolve_interface(&format!("  {}  ", lo.address)).unwrap(),
            lo.address
        );
        assert!(matches!(
            resolve_interface("203.0.113.77"),
            Err(DriverError::Config(_))
        ));
        assert!(matches!(
            resolve_interface("no-such-nic-xyz"),
            Err(DriverError::Config(_))
        ));
        assert!(resolve_interface("").is_err());
    }

    #[test]
    fn parallel_map_preserves_order_and_respects_cancel() {
        let items: Vec<u32> = (0..50).collect();
        let cancel = CancelFlag::new();
        let out = parallel_map(&items, 8, &cancel, |i| (i % 2 == 0).then_some(*i));
        assert_eq!(out, (0..50).filter(|i| i % 2 == 0).collect::<Vec<_>>());

        let cancel = CancelFlag::new();
        cancel.cancel();
        assert!(parallel_map(&items, 8, &cancel, |i| Some(*i)).is_empty());
        assert!(parallel_map::<u32, u32>(&[], 4, &CancelFlag::new(), |i| Some(*i)).is_empty());
    }

    #[test]
    fn device_text_is_neutralised() {
        assert_eq!(sanitize("  Lux\u{0}\r\n\u{7f}é-3  ", 64), "Lux.....-3");
        assert_eq!(sanitize(&"A".repeat(500), 10), "AAAAAAAAAA");
    }
}

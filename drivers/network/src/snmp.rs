//! SNMP discovery: ask each host's agent who it is.
//!
//! One GET per object (sysDescr, sysObjectID, sysName) over SNMPv2c. The agent
//! answering sysDescr is enough to report a device; the other two are added
//! when present. The community is a caller-supplied credential (`public` by
//! default) and is sent in clear text.

use std::net::SocketAddr;

use snmp2::{Oid, SyncSession, Value};

use tpt_app_av_commissioning_driver::{DiscoveredDevice, DriverError};

use crate::util::{parallel_map, sanitize, Ctx};

pub(crate) const DEFAULT_COMMUNITY: &str = "public";
pub(crate) const DEFAULT_PORT: u16 = 161;

const SYS_DESCR: [u64; 9] = [1, 3, 6, 1, 2, 1, 1, 1, 0];
const SYS_OBJECT_ID: [u64; 9] = [1, 3, 6, 1, 2, 1, 1, 2, 0];
const SYS_NAME: [u64; 9] = [1, 3, 6, 1, 2, 1, 1, 5, 0];

fn get_text(session: &mut SyncSession, oid: &[u64; 9]) -> Option<String> {
    let oid = Oid::from(oid).ok()?;
    let pdu = session.get(&oid).ok()?;
    if pdu.error_status != 0 {
        return None;
    }
    pdu.varbinds.into_iter().find_map(|(_, v)| match v {
        Value::OctetString(b) => Some(sanitize(&String::from_utf8_lossy(b), 160)),
        Value::ObjectIdentifier(o) => Some(o.to_string()),
        _ => None,
    })
}

pub(crate) fn probe(
    ctx: &Ctx<'_>,
    community: &str,
    port: u16,
) -> Result<Vec<DiscoveredDevice>, DriverError> {
    if community.is_empty() || community.len() > 64 || community.chars().any(char::is_control) {
        return Err(DriverError::Config(
            "community must be 1-64 printable characters".to_owned(),
        ));
    }
    Ok(parallel_map(
        ctx.hosts,
        ctx.concurrency,
        ctx.cancel,
        |host| {
            let target = SocketAddr::new(*host, port);
            let req_id = i32::from(port) + 1;
            let mut session =
                SyncSession::new_v2c(target, community.as_bytes(), Some(ctx.timeout), req_id)
                    .ok()?;
            let descr = get_text(&mut session, &SYS_DESCR)?;
            let mut device =
                DiscoveredDevice::new(target.to_string(), "snmp").detail("sys_descr", descr);
            if let Some(id) = get_text(&mut session, &SYS_OBJECT_ID) {
                device = device.detail("sys_object_id", id);
            }
            if let Some(name) = get_text(&mut session, &SYS_NAME) {
                device = device.detail("sys_name", name);
            }
            Some(device)
        },
    ))
}

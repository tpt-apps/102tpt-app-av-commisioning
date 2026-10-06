//! A small, strict DNS message codec for mDNS discovery.
//!
//! Replies come from arbitrary devices on the LAN, so parsing assumes hostile
//! input: every read is bounds-checked, name compression pointers are followed
//! at most [`MAX_JUMPS`] times, names are capped at 255 bytes, and a message
//! may carry at most [`MAX_RECORDS`] records. Anything else is an error, never
//! a panic or an unbounded loop.

use std::net::Ipv4Addr;

/// Compression pointers followed per name.
pub const MAX_JUMPS: usize = 16;
/// Records accepted per message.
pub const MAX_RECORDS: usize = 512;
/// Longest accepted name.
pub const MAX_NAME: usize = 255;

pub const TYPE_A: u16 = 1;
pub const TYPE_PTR: u16 = 12;
pub const TYPE_TXT: u16 = 16;
pub const TYPE_SRV: u16 = 33;

/// Record data this crate understands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RData {
    Ptr(String),
    Srv { port: u16, target: String },
    A(Ipv4Addr),
    Txt(Vec<String>),
    Other,
}

/// One resource record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DnsRecord {
    pub name: String,
    pub rtype: u16,
    pub data: RData,
}

/// A parsed message: the answer, authority and additional records together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DnsMessage {
    pub is_response: bool,
    pub records: Vec<DnsRecord>,
}

/// Encode a one-question query. `unicast_response` sets the mDNS "QU" bit so
/// responders reply to our port rather than to the multicast group.
pub fn encode_query(name: &str, qtype: u16, unicast_response: bool) -> Result<Vec<u8>, String> {
    let mut out = vec![0u8; 12];
    out[5] = 1; // QDCOUNT = 1
    if name.is_empty() || name.len() > MAX_NAME {
        return Err("name must be 1-255 characters".to_owned());
    }
    for label in name.trim_end_matches('.').split('.') {
        if label.is_empty() || label.len() > 63 {
            return Err(format!("invalid DNS label {label:?}"));
        }
        if !label
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(format!("invalid character in DNS label {label:?}"));
        }
        out.push(label.len() as u8);
        out.extend_from_slice(label.as_bytes());
    }
    out.push(0);
    out.extend_from_slice(&qtype.to_be_bytes());
    let class: u16 = if unicast_response { 0x8001 } else { 0x0001 };
    out.extend_from_slice(&class.to_be_bytes());
    Ok(out)
}

fn u16_at(p: &[u8], at: usize) -> Result<u16, String> {
    p.get(at..at + 2)
        .map(|b| u16::from_be_bytes([b[0], b[1]]))
        .ok_or_else(|| "truncated message".to_owned())
}

/// Read a (possibly compressed) name at `pos`. Returns the name and the
/// offset just past it in the *original* position (not following pointers).
fn read_name(p: &[u8], pos: usize) -> Result<(String, usize), String> {
    let mut name = String::new();
    let mut cursor = pos;
    let mut end: Option<usize> = None;
    let mut jumps = 0;
    loop {
        let len = *p.get(cursor).ok_or("truncated name")? as usize;
        match len {
            0 => {
                return Ok((name, end.unwrap_or(cursor + 1)));
            }
            l if l & 0xC0 == 0xC0 => {
                let low = *p.get(cursor + 1).ok_or("truncated pointer")? as usize;
                end.get_or_insert(cursor + 2);
                jumps += 1;
                if jumps > MAX_JUMPS {
                    return Err("too many compression pointers".to_owned());
                }
                cursor = ((l & 0x3F) << 8) | low;
            }
            l if l & 0xC0 != 0 => return Err("unsupported label type".to_owned()),
            l => {
                let label = p.get(cursor + 1..cursor + 1 + l).ok_or("truncated label")?;
                if !name.is_empty() {
                    name.push('.');
                }
                name.extend(label.iter().map(|b| {
                    if b.is_ascii_graphic() {
                        *b as char
                    } else {
                        '.'
                    }
                }));
                if name.len() > MAX_NAME {
                    return Err("name too long".to_owned());
                }
                cursor += 1 + l;
            }
        }
    }
}

/// Parse a DNS message.
pub fn parse(p: &[u8]) -> Result<DnsMessage, String> {
    if p.len() < 12 {
        return Err("shorter than a DNS header".to_owned());
    }
    let flags = u16_at(p, 2)?;
    let qd = u16_at(p, 4)? as usize;
    let counts = [u16_at(p, 6)?, u16_at(p, 8)?, u16_at(p, 10)?];
    let total: usize = counts.iter().map(|c| *c as usize).sum();
    if qd > 16 || total > MAX_RECORDS {
        return Err("message declares too many entries".to_owned());
    }
    let mut pos = 12;
    for _ in 0..qd {
        let (_, next) = read_name(p, pos)?;
        pos = next + 4;
        if pos > p.len() {
            return Err("truncated question".to_owned());
        }
    }
    let mut records = Vec::with_capacity(total);
    for _ in 0..total {
        let (name, next) = read_name(p, pos)?;
        let rtype = u16_at(p, next)?;
        let rdlen = u16_at(p, next + 8)? as usize;
        let start = next + 10;
        let rdata = p
            .get(start..start + rdlen)
            .ok_or_else(|| "truncated record data".to_owned())?;
        let data = match rtype {
            TYPE_PTR => RData::Ptr(read_name(p, start)?.0),
            TYPE_SRV if rdlen >= 6 => RData::Srv {
                port: u16_at(p, start + 4)?,
                target: read_name(p, start + 6)?.0,
            },
            TYPE_A if rdlen == 4 => RData::A(Ipv4Addr::new(rdata[0], rdata[1], rdata[2], rdata[3])),
            TYPE_TXT => {
                let mut strings = Vec::new();
                let mut i = 0;
                while i < rdata.len() {
                    let l = rdata[i] as usize;
                    let s = rdata
                        .get(i + 1..i + 1 + l)
                        .ok_or_else(|| "truncated TXT string".to_owned())?;
                    strings.push(
                        s.iter()
                            .map(|b| {
                                if b.is_ascii_graphic() || *b == b' ' {
                                    *b as char
                                } else {
                                    '.'
                                }
                            })
                            .collect(),
                    );
                    i += 1 + l;
                }
                RData::Txt(strings)
            }
            _ => RData::Other,
        };
        records.push(DnsRecord { name, rtype, data });
        pos = start + rdlen;
    }
    Ok(DnsMessage {
        is_response: flags & 0x8000 != 0,
        records,
    })
}

#[cfg(test)]
pub(crate) mod build {
    //! Helpers for building response packets in tests.
    pub fn name(n: &str) -> Vec<u8> {
        let mut out = Vec::new();
        for label in n.split('.') {
            out.push(label.len() as u8);
            out.extend_from_slice(label.as_bytes());
        }
        out.push(0);
        out
    }

    pub fn record(owner: &str, rtype: u16, rdata: &[u8]) -> Vec<u8> {
        let mut out = name(owner);
        out.extend_from_slice(&rtype.to_be_bytes());
        out.extend_from_slice(&0x8001u16.to_be_bytes());
        out.extend_from_slice(&120u32.to_be_bytes());
        out.extend_from_slice(&(rdata.len() as u16).to_be_bytes());
        out.extend_from_slice(rdata);
        out
    }

    pub fn message(records: &[Vec<u8>]) -> Vec<u8> {
        let mut out = vec![0, 0, 0x84, 0, 0, 0];
        out.extend_from_slice(&(records.len() as u16).to_be_bytes());
        out.extend_from_slice(&[0, 0, 0, 0]);
        for r in records {
            out.extend_from_slice(r);
        }
        out
    }

    pub fn srv(port: u16, target: &str) -> Vec<u8> {
        let mut out = vec![0, 0, 0, 0];
        out.extend_from_slice(&port.to_be_bytes());
        out.extend_from_slice(&name(target));
        out
    }

    pub fn txt(strings: &[&str]) -> Vec<u8> {
        strings
            .iter()
            .flat_map(|s| std::iter::once(s.len() as u8).chain(s.bytes()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::build::*;
    use super::*;

    #[test]
    fn query_encoding() {
        let q = encode_query("_http._tcp.local", TYPE_PTR, true).unwrap();
        assert_eq!(&q[..12], &[0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
        assert_eq!(&q[12..18], &[5, b'_', b'h', b't', b't', b'p']);
        assert_eq!(&q[q.len() - 4..], &[0, 12, 0x80, 1]);
        let plain = encode_query("a.local", TYPE_A, false).unwrap();
        assert_eq!(&plain[plain.len() - 2..], &[0, 1]);
        for bad in ["", "a..b", "a b.local", &"x".repeat(64), &"a.".repeat(200)] {
            assert!(encode_query(bad, TYPE_PTR, false).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn parses_ptr_srv_a_and_txt() {
        let msg = message(&[
            record("_http._tcp.local", TYPE_PTR, &name("Lux._http._tcp.local")),
            record("Lux._http._tcp.local", TYPE_SRV, &srv(8080, "lux.local")),
            record("lux.local", TYPE_A, &[192, 168, 1, 50]),
            record(
                "Lux._http._tcp.local",
                TYPE_TXT,
                &txt(&["model=Lux-3", "fw=1.2"]),
            ),
            record("x.local", 99, &[1, 2, 3]),
        ]);
        let m = parse(&msg).unwrap();
        assert!(m.is_response);
        assert_eq!(m.records.len(), 5);
        assert_eq!(m.records[0].data, RData::Ptr("Lux._http._tcp.local".into()));
        assert_eq!(
            m.records[1].data,
            RData::Srv {
                port: 8080,
                target: "lux.local".into()
            }
        );
        assert_eq!(m.records[2].data, RData::A(Ipv4Addr::new(192, 168, 1, 50)));
        assert_eq!(
            m.records[3].data,
            RData::Txt(vec!["model=Lux-3".into(), "fw=1.2".into()])
        );
        assert_eq!(m.records[4].data, RData::Other);
    }

    #[test]
    fn follows_compression_pointers() {
        // A PTR whose rdata is a pointer back to the owner name at offset 12.
        let mut msg = vec![0, 0, 0x84, 0, 0, 0, 0, 1, 0, 0, 0, 0];
        msg.extend_from_slice(&name("a.local")); // offset 12
        msg.extend_from_slice(&TYPE_PTR.to_be_bytes());
        msg.extend_from_slice(&1u16.to_be_bytes());
        msg.extend_from_slice(&0u32.to_be_bytes());
        msg.extend_from_slice(&2u16.to_be_bytes());
        msg.extend_from_slice(&[0xC0, 12]);
        let m = parse(&msg).unwrap();
        assert_eq!(m.records[0].name, "a.local");
        assert_eq!(m.records[0].data, RData::Ptr("a.local".into()));
    }

    #[test]
    fn hostile_packets_are_errors_not_panics_or_loops() {
        // A pointer loop.
        let mut looped = vec![0, 0, 0x84, 0, 0, 0, 0, 1, 0, 0, 0, 0];
        looped.extend_from_slice(&[0xC0, 12, 0, 12, 0, 1, 0, 0, 0, 0, 0, 0]);
        assert!(parse(&looped).is_err());
        // Every kind of truncation of a valid message.
        let good = message(&[
            record("_x._tcp.local", TYPE_PTR, &name("i._x._tcp.local")),
            record("i._x._tcp.local", TYPE_SRV, &srv(1, "h.local")),
            record("i._x._tcp.local", TYPE_TXT, &txt(&["a=b"])),
        ]);
        assert!(parse(&good).is_ok());
        for cut in 0..good.len() {
            let _ = parse(&good[..cut]); // must not panic
        }
        // Declared counts beyond the limit.
        let mut huge = vec![0u8; 12];
        huge[6] = 0xff;
        huge[7] = 0xff;
        assert!(parse(&huge).is_err());
        // Unsupported label type, oversized name, bad TXT length.
        let mut bad_label = vec![0, 0, 0x84, 0, 0, 0, 0, 1, 0, 0, 0, 0];
        bad_label.push(0x80);
        assert!(parse(&bad_label).is_err());
        let long = message(&[record(
            &vec!["a".repeat(60); 6].join("."),
            TYPE_A,
            &[1, 2, 3, 4],
        )]);
        assert!(parse(&long).is_err());
        let bad_txt = message(&[record("t.local", TYPE_TXT, &[9, b'a'])]);
        assert!(parse(&bad_txt).is_err());
        assert!(parse(&[]).is_err());
        assert!(parse(&[0; 11]).is_err());
    }

    #[test]
    fn device_text_cannot_smuggle_control_bytes() {
        let msg = message(&[record("t.local", TYPE_TXT, &txt(&["a\u{1b}[31m=b"]))]);
        let m = parse(&msg).unwrap();
        assert_eq!(m.records[0].data, RData::Txt(vec!["a.[31m=b".into()]));
    }
}

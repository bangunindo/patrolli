// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Cisco IOS, IOS XR, NX-OS, ASA and FTD messages.
//!
//! ```text
//! IOS    <189>521: rtr-lab-01: *Jan 12 03:04:05.678: %LINK-5-CHANGED: Interface Gi2, changed state …
//! IOS XR <149>2647599: rtr-lab-02 RP/0/RSP0/CPU0:Jan 12 03:04:05.678 UTC: bgp[1051]: %ROUTING-BGP-5-MAXPFX : …
//! NX-OS  <190>sw-lab-01: 2026 Jan 12 03:04:05 UTC: %ETHPORT-5-IF_UP: Interface Ethernet1/1 is up
//! ASA    <166>Jan 12 2026 03:04:05 asa-lab-01 : %ASA-6-302013: Built outbound TCP connection …
//! FTD    <166>Jan 12 2026 03:04:05 fw-lab-01 %FTD-6-430003: AccessControlRuleAction: Allow, SrcIP: …
//! ```
//!
//! Every platform writes a tag, `%FACILITY[-SUBFACILITY]-SEVERITY-MNEMONIC:`, found by [`find`].
//! What comes before it differs per platform and per configuration (message counter, sequence
//! number, origin hostname, IOS XR node and process, the device's own timestamp); [`preamble`]
//! picks out what it recognises and ignores the rest. FTD connection events carry `Key: value`
//! pairs, which [`pairs`] splits.

use super::time::parse_time;
use crate::payload::{Value, ZString};
use chrono::{DateTime, Utc};
use opentelemetry_proto::tonic::logs::v1::SeverityNumber;

/// How far into a message the tag may start. The longest preamble (IOS XR) is about 100 bytes;
/// past this, a `%TAG:` is free text inside some other message.
const SEARCH_WINDOW: usize = 512;
const LINE_END: [char; 3] = ['\r', '\n', '\0'];
const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];

pub(crate) struct Tag<'a> {
    /// Byte offset of the `%`.
    pub at: usize,
    pub facility: &'a str,
    pub subfacility: Option<&'a str>,
    /// 0 (emergency) to 7 (debug), as syslog.
    pub severity: u8,
    pub mnemonic: &'a str,
    /// After the colon, without leading spaces or the line ending.
    pub text: &'a str,
}

/// Finds the tag: a `%` at the start of `packet`, right after the PRI, or after a space or colon,
/// within the first [`SEARCH_WINDOW`] bytes.
pub(crate) fn find(packet: &str) -> Option<Tag<'_>> {
    let window = &packet.as_bytes()[..packet.len().min(SEARCH_WINDOW)];
    let mut from = 0;
    while let Some(pos) = memchr::memchr(b'%', &window[from..]) {
        let at = from + pos;
        if (at == 0 || matches!(window[at - 1], b' ' | b':' | b'>'))
            && let Some(tag) = tag_at(packet, at)
        {
            return Some(tag);
        }
        from = at + 1;
    }
    None
}

fn tag_at(packet: &str, at: usize) -> Option<Tag<'_>> {
    let rest = &packet[at + 1..];
    let end = rest
        .bytes()
        .position(|b| !(b.is_ascii_alphanumeric() || b == b'_' || b == b'-'))?;
    let name = &rest[..end];
    // IOS XR puts a space before the colon.
    let after = &rest[end..];
    let text = after.strip_prefix(' ').unwrap_or(after).strip_prefix(':')?;
    let mut parts = name.split('-');
    let facility = parts.next()?;
    let (subfacility, severity, mnemonic) = match (parts.next(), parts.next(), parts.next()) {
        (Some(sev), Some(mnemonic), None) => (None, sev, mnemonic),
        (Some(sub), Some(sev), Some(mnemonic)) if parts.next().is_none() => {
            (Some(sub), sev, mnemonic)
        }
        _ => return None,
    };
    let severity = match severity.as_bytes() {
        [d @ b'0'..=b'7'] => d - b'0',
        _ => return None,
    };
    if !facility.starts_with(|c: char| c.is_ascii_alphabetic())
        || mnemonic.is_empty()
        || subfacility.is_some_and(str::is_empty)
    {
        return None;
    }
    Some(Tag {
        at,
        facility,
        subfacility,
        severity,
        mnemonic,
        text: text.trim_start_matches(' ').trim_end_matches(LINE_END),
    })
}

#[derive(Default)]
pub(crate) struct Preamble<'a> {
    /// The last counter: the sequence number with `service sequence-numbers`, else the
    /// message counter.
    pub seq: Option<u64>,
    /// `logging origin-id`, or the NX-OS hostname after its timestamp.
    pub host: Option<&'a str>,
    /// IOS XR node, `RP/0/RSP0/CPU0`.
    pub node: Option<&'a str>,
    pub process: Option<&'a str>,
    pub pid: Option<i64>,
    /// The device's timestamp, when it parsed and its zone resolved.
    pub time: Option<DateTime<Utc>>,
    /// The timestamp carried `*` (clock never set) or `.` (clock lost its time source).
    pub unsynced: bool,
}

/// Reads what precedes the tag: segments separated by `": "`, each a counter, a hostname, a
/// `process[pid]` or a timestamp. Anything else is skipped.
pub(crate) fn preamble(s: &str) -> Preamble<'_> {
    let mut p = Preamble::default();
    let mut rest = Some(s);
    let segments = std::iter::from_fn(|| {
        let r = rest?;
        match find_sep(r, 0, b':') {
            Some(at) => {
                rest = Some(&r[at + 2..]);
                Some(&r[..at])
            }
            None => rest.take(),
        }
    });
    for seg in segments {
        let seg = seg.trim_matches([' ', ':']);
        if seg.is_empty() {
            continue;
        }
        if is_digits(seg) {
            p.seq = seg.parse().ok();
        } else if let Some((name, pid)) = process(seg) {
            p.process = Some(name);
            p.pid = Some(pid);
        } else if !timestamp(seg, &mut p) && !seg.contains(' ') && p.host.is_none() {
            p.host = Some(seg);
        }
    }
    p
}

/// The first `sep` followed by a space at or after `from`: `": "`, `", "`. A byte search, which
/// is cheaper on short text than setting up `str::find`'s substring searcher.
fn find_sep(s: &str, mut from: usize, sep: u8) -> Option<usize> {
    let b = s.as_bytes();
    while let Some(p) = memchr::memchr(sep, &b[from..]) {
        let at = from + p;
        if b.get(at + 1) == Some(&b' ') {
            return Some(at);
        }
        from = at + 1;
    }
    None
}

fn is_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// `bgp[1051]`.
fn process(seg: &str) -> Option<(&str, i64)> {
    let (name, pid) = seg.strip_suffix(']')?.split_once('[')?;
    if name.is_empty() || name.contains(' ') || !is_digits(pid) {
        return None;
    }
    Some((name, pid.parse().ok()?))
}

/// Words of `s` with their byte offsets.
fn words(s: &str) -> impl Iterator<Item = (usize, &str)> {
    s.split(' ')
        .scan(0, |off, w| {
            let at = *off;
            *off += w.len() + 1;
            Some((at, w))
        })
        .filter(|(_, w)| !w.is_empty())
}

fn is_month(w: &str) -> bool {
    w.len() == 3 && MONTHS.iter().any(|m| w.eq_ignore_ascii_case(m))
}

/// A zone after the time: `UTC`, `WIB`, `+07:00`, `Asia/Jakarta`.
fn is_zone(w: &str) -> bool {
    let b = w.as_bytes();
    (2..=5).contains(&b.len()) && b.iter().all(u8::is_ascii_uppercase)
        || matches!(b.first(), Some(b'+' | b'-'))
            && b[1..].iter().all(|c| c.is_ascii_digit() || *c == b':')
        || w.contains('/')
}

/// Finds a timestamp in `seg`, `[*|.]Mmm dd [yyyy] hh:mm:ss[.fff] [zone]` or
/// `yyyy Mmm dd hh:mm:ss[.fff] [zone]`, and fills in what surrounds it: an IOS XR node glued in
/// front (`RP/0/RSP0/CPU0:Jan …`), a hostname before that, or a hostname after it (NX-OS).
fn timestamp<'a>(seg: &'a str, p: &mut Preamble<'a>) -> bool {
    let mut all = words(seg).peekable();
    while let Some((start, word)) = all.next() {
        let (node, glue) = match word.rsplit_once(':') {
            Some((node, _)) if node.contains('/') => (Some(node), node.len() + 1),
            _ => (None, 0),
        };
        let lead = &seg[start + glue..];
        let unsynced = lead.starts_with(['*', '.']);
        let lead = lead.trim_start_matches(['*', '.']);
        let first = lead.split(' ').next().unwrap_or_default();
        let starts = is_month(first)
            || first.len() == 4
                && is_digits(first)
                && all.peek().is_some_and(|(_, next)| is_month(next));
        if !starts {
            continue;
        }
        let Some(time_end) = time_end(lead) else {
            continue;
        };
        // A zone may follow the time, then (NX-OS) the hostname.
        let mut stamp_end = time_end;
        let mut after = words(&lead[time_end..]);
        let host = match after.next() {
            Some((at, w)) if is_zone(w) => {
                stamp_end = time_end + at + w.len();
                after.next().map(|(_, w)| w)
            }
            other => other.map(|(_, w)| w),
        };
        p.time = parse_time(&lead[..stamp_end]);
        p.unsynced = unsynced;
        p.node = node;
        if p.host.is_none() {
            let before = seg[..start].trim();
            p.host = host.or((!before.is_empty() && !before.contains(' ')).then_some(before));
        }
        return true;
    }
    false
}

/// Where the `hh:mm:ss[.fff]` word among the first few of `s` ends.
fn time_end(s: &str) -> Option<usize> {
    words(s).take(4).find_map(|(at, w)| {
        let b = w.as_bytes();
        (b.len() >= 8
            && b[0].is_ascii_digit()
            && b[1].is_ascii_digit()
            && b[2] == b':'
            && b[5] == b':')
            .then_some(at + w.len())
    })
}

/// FTD/NGIPS connection and intrusion events (430001–430005): `Key: value, Key: value`. A `", "`
/// separates pairs only when a `Key: ` follows it, so a value holding `", "` stays whole.
pub(crate) fn pairs(text: &str) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    let Some((mut key, mut value_at)) = key_at(text, 0) else {
        return out;
    };
    let mut from = value_at;
    loop {
        match find_sep(text, from, b',') {
            Some(sep) => match key_at(text, sep + 2) {
                Some((next_key, next_value)) => {
                    out.push((key, &text[value_at..sep]));
                    (key, value_at, from) = (next_key, next_value, next_value);
                }
                None => from = sep + 2,
            },
            None => {
                out.push((key, text[value_at..].trim_end_matches([',', ' '])));
                return out;
            }
        }
    }
}

/// A `Key: ` at `at`, and where its value starts.
fn key_at(text: &str, at: usize) -> Option<(&str, usize)> {
    let rest = &text[at..];
    let end = rest
        .bytes()
        .position(|b| !(b.is_ascii_alphanumeric() || b == b'_'))?;
    if end == 0 || !rest.as_bytes()[0].is_ascii_alphabetic() || !rest[end..].starts_with(": ") {
        return None;
    }
    Some((&rest[..end], at + end + 2))
}

/// The FTD/NGIPS events whose text is [`pairs`].
fn has_pairs(tag: &Tag<'_>) -> bool {
    matches!(tag.facility, "FTD" | "NGIPS")
        && matches!(
            tag.mnemonic,
            "430001" | "430002" | "430003" | "430004" | "430005"
        )
}

/// The syslog scale, mapped as the syslog listener maps the PRI severity.
fn severity_number(sev: u8) -> Option<SeverityNumber> {
    Some(match sev {
        0 => SeverityNumber::Fatal3,
        1 => SeverityNumber::Fatal2,
        2 => SeverityNumber::Fatal,
        3 => SeverityNumber::Error,
        4 => SeverityNumber::Warn,
        5 => SeverityNumber::Info2,
        6 => SeverityNumber::Info,
        7 => SeverityNumber::Debug,
        _ => return None,
    })
}

pub(crate) struct CiscoAttrs<'a> {
    /// The `KvList` for attribute `cisco`.
    pub value: Value<'a>,
    /// The device's timestamp, unless its clock was not synced.
    pub time: Option<DateTime<Utc>>,
    pub severity: Option<SeverityNumber>,
}

pub(crate) fn into_attrs<'a>(tag: &Tag<'a>, pre: &Preamble<'a>) -> CiscoAttrs<'a> {
    let str = |k: &'static str, v: &'a str| {
        (ZString::from_static(k), Value::String(ZString::from_ref(v)))
    };
    let int = |k: &'static str, v: i64| (ZString::from_static(k), Value::Int64(v));
    let mut kv = Vec::with_capacity(12);
    kv.push(str("facility", tag.facility));
    if let Some(sub) = tag.subfacility {
        kv.push(str("subfacility", sub));
    }
    kv.push(int("severity", tag.severity as i64));
    kv.push(str("mnemonic", tag.mnemonic));
    kv.push(str("text", tag.text));
    if let Some(seq) = pre.seq {
        kv.push(int("seq", seq as i64));
    }
    if let Some(node) = pre.node {
        kv.push(str("node", node));
    }
    if let Some(process) = pre.process {
        kv.push(str("process", process));
    }
    if let Some(pid) = pre.pid {
        kv.push(int("pid", pid));
    }
    if pre.unsynced {
        kv.push((ZString::from_static("clock_synced"), Value::Bool(false)));
    }
    if has_pairs(tag) {
        let attrs: Vec<Value<'a>> = pairs(tag.text)
            .into_iter()
            .map(|(k, v)| Value::KvList(vec![str("key", k), str("value", v)]))
            .collect();
        if !attrs.is_empty() {
            kv.push((ZString::from_static("attrs"), Value::Array(attrs)));
        }
    }
    CiscoAttrs {
        value: Value::KvList(kv),
        time: pre.time.filter(|_| !pre.unsynced),
        severity: severity_number(tag.severity),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(packet: &str) -> Option<(&str, Option<&str>, u8, &str, &str)> {
        find(packet).map(|t| (t.facility, t.subfacility, t.severity, t.mnemonic, t.text))
    }

    #[test]
    fn tags_per_platform() {
        assert_eq!(
            tag(
                "<189>521: rtr-lab-01: *Jan 12 03:04:05.678: %LINK-5-CHANGED: Interface Gi2, changed state to down\n"
            ),
            Some((
                "LINK",
                None,
                5,
                "CHANGED",
                "Interface Gi2, changed state to down"
            ))
        );
        assert_eq!(
            tag(
                "<149>12: rtr-lab-02 RP/0/RSP0/CPU0:Jan 12 03:04:05.678 UTC: bgp[1051]: %ROUTING-BGP-5-MAXPFX : No. of prefixes reached 90"
            ),
            Some((
                "ROUTING",
                Some("BGP"),
                5,
                "MAXPFX",
                "No. of prefixes reached 90"
            ))
        );
        assert_eq!(
            tag(
                "<166>Jan 12 2026 03:04:05 asa-lab-01 : %ASA-6-302013: Built outbound TCP connection 7"
            ),
            Some(("ASA", None, 6, "302013", "Built outbound TCP connection 7"))
        );
        assert_eq!(
            tag("<166>%ASA-session-6-302013: Built"),
            Some(("ASA", Some("session"), 6, "302013", "Built"))
        );
        assert_eq!(
            tag(
                "<190>2026 Jan 12 03:04:05 sw-lab-01 %$ VDC-1 %$ %ETHPORT-5-IF_UP: Interface Ethernet1/1 is up"
            ),
            Some(("ETHPORT", None, 5, "IF_UP", "Interface Ethernet1/1 is up"))
        );
        assert_eq!(
            tag("%SYS-5-CONFIG_I:"),
            Some(("SYS", None, 5, "CONFIG_I", ""))
        );
        assert_eq!(find("<189>12: %SYS-5-CONFIG_I: x").unwrap().at, 9);
    }

    #[test]
    fn tag_rejects() {
        assert!(find("<13>Jan 12 03:04:05 host app: printf %d: value").is_none());
        assert!(find("<13>Jan 12 03:04:05 host app: %SYS-9-CONFIG_I: bad severity").is_none());
        assert!(find("<13>Jan 12 03:04:05 host app: %SYS-5-CONFIG_I no colon").is_none());
        assert!(find("<13>Jan 12 03:04:05 host app: 100%SYS-5-CONFIG_I: glued").is_none());
        assert!(find("<13>Jan 12 03:04:05 host app: %A-B-C-5-X: too many parts").is_none());
        assert!(find("<13>Jan 12 03:04:05 host app: %1X-5-Y: digit first").is_none());
        let far = format!(
            "<13>Jan 12 03:04:05 host app: {} %SYS-5-CONFIG_I: x",
            "a".repeat(600)
        );
        assert!(find(&far).is_none());
    }

    #[test]
    fn ios_preamble_unsynced() {
        let p = preamble("521: rtr-lab-01: *Jan 12 03:04:05.678: ");
        assert_eq!(p.seq, Some(521));
        assert_eq!(p.host, Some("rtr-lab-01"));
        assert!(p.unsynced);
        // No zone: local time.
        assert!(p.time.is_some());
    }

    #[test]
    fn ios_preamble_two_counters() {
        let p = preamble("78: 000096: Jan 12 2026 03:04:05.678 UTC: ");
        assert_eq!(p.seq, Some(96));
        assert_eq!(p.host, None);
        assert!(!p.unsynced);
        assert_eq!(
            p.time.unwrap().to_rfc3339(),
            "2026-01-12T03:04:05.678+00:00"
        );
    }

    #[test]
    fn iosxr_preamble() {
        let p = preamble("12: rtr-lab-02 RP/0/RSP0/CPU0:Jan 12 2026 03:04:05.678 UTC: bgp[1051]: ");
        assert_eq!(p.seq, Some(12));
        assert_eq!(p.host, Some("rtr-lab-02"));
        assert_eq!(p.node, Some("RP/0/RSP0/CPU0"));
        assert_eq!((p.process, p.pid), (Some("bgp"), Some(1051)));
        assert_eq!(
            p.time.unwrap().to_rfc3339(),
            "2026-01-12T03:04:05.678+00:00"
        );

        // No hostname, no zone, padded day, a space before the colon.
        let p = preamble("94307: LC/0/2/CPU0:Jul  7 2026 20:16:14.834 : ifmgr[214]: ");
        assert_eq!(p.host, None);
        assert_eq!(p.node, Some("LC/0/2/CPU0"));
        assert_eq!((p.process, p.pid), (Some("ifmgr"), Some(214)));
        assert!(p.time.is_some());
    }

    #[test]
    fn nxos_preambles() {
        let p = preamble("sw-lab-01: 2026 Jan 12 03:04:05 UTC: ");
        assert_eq!(p.host, Some("sw-lab-01"));
        assert_eq!(p.time.unwrap().to_rfc3339(), "2026-01-12T03:04:05+00:00");

        let p = preamble("2026 Jan 12 03:04:05 sw-lab-02 ");
        assert_eq!(p.host, Some("sw-lab-02"));
        assert!(p.time.is_some());

        // The VDC marker is not a hostname.
        let p = preamble(": 2026 Jan 12 03:04:05 UTC: %$ VDC-1 %$ ");
        assert_eq!(p.host, None);
        assert!(p.time.is_some());
    }

    #[test]
    fn unknown_zone_drops_the_time() {
        let p = preamble("5: Jan 12 2026 03:04:05 XYZ: ");
        assert_eq!(p.seq, Some(5));
        assert!(p.time.is_none());
        assert!(!p.unsynced);
    }

    #[test]
    fn empty_and_junk_preambles() {
        let p = preamble("");
        assert!(p.seq.is_none() && p.host.is_none() && p.time.is_none());
        let p = preamble("some words here: more words: ");
        assert!(p.host.is_none() && p.time.is_none());
    }

    #[test]
    fn ftd_pairs() {
        assert_eq!(
            pairs(
                "AccessControlRuleAction: Allow, SrcIP: 192.0.2.1, DstIP: 198.51.100.2, URL: http://example.com/?a=1, b=2, Protocol: tcp,"
            ),
            vec![
                ("AccessControlRuleAction", "Allow"),
                ("SrcIP", "192.0.2.1"),
                ("DstIP", "198.51.100.2"),
                ("URL", "http://example.com/?a=1, b=2"),
                ("Protocol", "tcp"),
            ]
        );
        assert_eq!(pairs("Rule: , Next: x"), vec![("Rule", ""), ("Next", "x")]);
        assert!(pairs("Built outbound TCP connection").is_empty());
    }

    #[test]
    fn attrs_layout() {
        let packet = "<166>Jan 12 2026 03:04:05 fw-lab-01 %FTD-6-430003: AccessControlRuleAction: Allow, SrcIP: 192.0.2.1";
        let t = find(packet).unwrap();
        let pre = preamble("7: *Jan 12 2026 03:04:05 UTC: ");
        let a = into_attrs(&t, &pre);
        let Value::KvList(kv) = &a.value else {
            panic!("not a kvlist");
        };
        let keys: Vec<_> = kv.iter().map(|(k, _)| k.to_string()).collect();
        assert_eq!(
            keys,
            [
                "facility",
                "severity",
                "mnemonic",
                "text",
                "seq",
                "clock_synced",
                "attrs"
            ]
        );
        assert_eq!(a.severity, Some(SeverityNumber::Info));
        // The clock was not synced, so its time is not used.
        assert!(a.time.is_none());

        // An ASA message keeps its text and gets no pairs.
        let t =
            find("<166>%ASA-4-106023: Deny tcp src outside:192.0.2.1/1 dst inside:198.51.100.2/22")
                .unwrap();
        let a = into_attrs(&t, &Preamble::default());
        let Value::KvList(kv) = &a.value else {
            panic!("not a kvlist");
        };
        assert!(kv.iter().all(|(k, _)| k.to_string() != "attrs"));
        assert_eq!(a.severity, Some(SeverityNumber::Warn));
    }
}

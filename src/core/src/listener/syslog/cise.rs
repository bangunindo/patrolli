// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Cisco ISE (and ACS 5.x) syslog parser.
//!
//! ```text
//! <pri>Mmm dd hh:mm:ss host CISE_<category> <msg_id> <total_seg> <seg_num> <segment>
//! segment 0: YYYY-MM-DD hh:mm:ss.fff ±hh:mm <sequence_num> <code> <SEV> <class>: <text>, k=v, k=v,
//! segment N:  k=v, k=v,
//! ```
//!
//! An event longer than the target's maximum length is split into segments that share a
//! `msg_id`; in arrival order, their bodies concatenate into the whole event. [`Reassembler`]
//! collects them. [`parse`] cuts an event into its header and `k=v` attributes; keys are free text
//! (`Device IP Address`) and repeat (`Step`), so [`Cise::into_attrs`] lists them as `{key, value}`
//! entries rather than turning them into attribute names.

use crate::config;
use crate::payload::{Value, ZString};
use chrono::{DateTime, Utc};
use opentelemetry_proto::tonic::logs::v1::SeverityNumber;
use rustc_hash::FxHashMap;
use smol_str::SmolStr;
use std::borrow::Cow;
use std::collections::VecDeque;
use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};

/// Tag prefixes: ISE, then the older ACS, which uses the same layout.
const PREFIXES: [&str; 2] = ["CISE_", "CSCOacs_"];
const LINE_END: [char; 3] = ['\r', '\n', '\0'];

/// The category in an ISE/ACS tag: `CISE_Passed_Authentications` → `Passed_Authentications`.
pub(crate) fn category(tag: &str) -> Option<&str> {
    PREFIXES
        .iter()
        .find_map(|p| tag.strip_prefix(p))
        .filter(|c| !c.is_empty())
}

pub(crate) struct Segment<'a> {
    pub msg_id: u64,
    pub total: u32,
    pub num: u32,
    /// From `msg_id` to the end, without the line ending.
    pub record: &'a str,
    /// What follows `<seg_num> `.
    pub body: &'a str,
}

/// Reads the counters that follow the tag. `after_tag` may start with spaces.
pub(crate) fn segment(after_tag: &str) -> Option<Segment<'_>> {
    let record = after_tag.trim_start_matches(' ').trim_end_matches(LINE_END);
    let (msg_id, rest) = counter(record)?;
    let (total, rest) = counter(rest)?;
    let (num, body) = counter(rest)?;
    let (total, num) = (u32::try_from(total).ok()?, u32::try_from(num).ok()?);
    if num >= total {
        return None;
    }
    Some(Segment {
        msg_id,
        total,
        num,
        record,
        body,
    })
}

/// A decimal followed by one space (or the end), and what follows that space.
fn counter(s: &str) -> Option<(u64, &str)> {
    let end = s
        .bytes()
        .position(|b| !b.is_ascii_digit())
        .unwrap_or(s.len());
    if end == 0 {
        return None;
    }
    let rest = match &s[end..] {
        "" => "",
        rest => rest.strip_prefix(' ')?,
    };
    Some((s[..end].parse().ok()?, rest))
}

#[derive(Default)]
pub(crate) struct Cise<'a> {
    pub time: Option<DateTime<Utc>>,
    pub sequence_num: Option<i64>,
    pub code: Option<i64>,
    pub severity: Option<&'a str>,
    pub class: Option<&'a str>,
    pub text: Option<Cow<'a, str>>,
    /// In arrival order, repeats kept.
    pub attrs: Vec<(Cow<'a, str>, Cow<'a, str>)>,
    /// The pairs of `Response={k=v; k=v; }`.
    pub response: Vec<(Cow<'a, str>, Cow<'a, str>)>,
}

/// Parses an event (or what arrived of it) from just after its segment counters.
pub(crate) fn parse(body: &str) -> Cise<'_> {
    let mut cise = Cise::default();
    let has_header = starts_with_date(body);
    let rest = if has_header {
        header(&mut cise, body)
    } else {
        body
    };
    let mut chunks = split(rest).into_iter();
    if has_header && let Some(first) = chunks.next() {
        let first = first.trim();
        match first.split_once(": ") {
            Some((class, text)) if !class.contains(' ') => {
                cise.class = Some(class);
                cise.text = Some(unescape(text));
            }
            _ => match first.strip_suffix(':') {
                Some(class) if !class.contains(' ') => cise.class = Some(class),
                _ => cise.text = Some(unescape(first)),
            },
        }
    }
    for chunk in chunks {
        let pair = chunk.trim_start();
        if pair.trim_end().is_empty() {
            continue;
        }
        match pair.split_once('=') {
            Some((key, value)) if !key.trim().is_empty() => {
                let key = key.trim();
                if key == "Response"
                    && let Some(inner) = value
                        .trim()
                        .strip_prefix('{')
                        .and_then(|v| v.strip_suffix('}'))
                {
                    response(inner, &mut cise.response);
                } else {
                    cise.attrs.push((unescape(key), unescape(value)));
                }
            }
            // A comma inside a value, not a separator.
            _ => {
                let target = match cise.attrs.last_mut() {
                    Some((_, value)) => value,
                    None => cise.text.get_or_insert(Cow::Borrowed("")),
                };
                if !target.is_empty() {
                    target.to_mut().push(',');
                }
                target.to_mut().push_str(&unescape(chunk));
            }
        }
    }
    cise
}

fn starts_with_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 10
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[7] == b'-'
        && b[8..10].iter().all(u8::is_ascii_digit)
}

/// The word at the start of `s` and what follows the spaces after it.
fn token(s: &str) -> (&str, &str) {
    match s.split_once(' ') {
        Some((tok, rest)) => (tok, rest.trim_start_matches(' ')),
        None => (s, ""),
    }
}

/// Reads `timestamp sequence_num [code] SEVERITY` and returns what follows. Alarms carry no
/// code, so every field after the timestamp is taken only when it looks like itself.
fn header<'a>(cise: &mut Cise<'a>, s: &'a str) -> &'a str {
    let (_, rest) = token(s);
    let (_, rest) = token(rest);
    let (_, rest) = token(rest);
    cise.time = parse_time(s[..s.len() - rest.len()].trim_end());
    let mut rest = rest;
    let (tok, next) = token(rest);
    if is_digits(tok) {
        cise.sequence_num = tok.parse().ok();
        rest = next;
    }
    let (tok, next) = token(rest);
    if is_digits(tok) {
        cise.code = tok.parse().ok();
        rest = next;
    }
    let (tok, next) = token(rest);
    if !tok.is_empty() && tok.bytes().all(|b| b.is_ascii_uppercase()) {
        cise.severity = Some(tok);
        rest = next;
    }
    rest
}

fn is_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// `2026-01-12 03:04:05.678 +07:00`. The guide spells the fraction `hh:mm:ss:xxx`, which ISE
/// does not send.
fn parse_time(s: &str) -> Option<DateTime<Utc>> {
    ["%Y-%m-%d %H:%M:%S%.f %:z", "%Y-%m-%d %H:%M:%S%.f %z"]
        .iter()
        .find_map(|fmt| DateTime::parse_from_str(s, fmt).ok())
        .map(|t| t.with_timezone(&Utc))
}

/// Splits on the commas that separate pairs: not escaped (`\,`), and not inside a `{...}` or
/// `[...]` that opens a value (`Response={...}`, `CmdSet=[ ... ]`).
fn split(s: &str) -> Vec<&str> {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut start = 0;
    let mut depth = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        if depth == 0 {
            // Outside brackets only these three matter: jump to the next one.
            let Some(at) = memchr::memchr3(b'\\', b'=', b',', &bytes[i..]) else {
                break;
            };
            i += at;
        }
        match bytes[i] {
            b'\\' => {
                i += 2;
                continue;
            }
            b'=' if depth == 0 && matches!(bytes.get(i + 1), Some(b'{' | b'[')) => {
                depth = 1;
                i += 2;
                continue;
            }
            b'{' | b'[' if depth > 0 => depth += 1,
            b'}' | b']' if depth > 0 => depth -= 1,
            b',' if depth == 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    out.push(&s[start..]);
    out
}

/// `k=v; k=v; ` into `out`.
fn response<'a>(inner: &'a str, out: &mut Vec<(Cow<'a, str>, Cow<'a, str>)>) {
    for pair in inner.split(';') {
        let pair = pair.trim();
        if pair.is_empty() {
            continue;
        }
        match pair.split_once('=') {
            Some((key, value)) if !key.trim().is_empty() => {
                out.push((unescape(key.trim()), unescape(value)));
            }
            _ => match out.last_mut() {
                Some((_, value)) => {
                    let value = value.to_mut();
                    value.push(';');
                    value.push_str(pair);
                }
                None => out.push((Cow::Borrowed(""), Cow::Borrowed(pair))),
            },
        }
    }
}

fn unescape(s: &str) -> Cow<'_, str> {
    if memchr::memchr(b'\\', s.as_bytes()).is_some() && s.contains("\\,") {
        Cow::Owned(s.replace("\\,", ","))
    } else {
        Cow::Borrowed(s)
    }
}

/// ISE severity to the OpenTelemetry scale, as the syslog listener maps the syslog levels.
fn severity_number(sev: &str) -> Option<SeverityNumber> {
    let is = |name: &str| sev.eq_ignore_ascii_case(name);
    if is("fatal") {
        Some(SeverityNumber::Fatal)
    } else if is("error") {
        Some(SeverityNumber::Error)
    } else if is("warn") || is("warning") {
        Some(SeverityNumber::Warn)
    } else if is("notice") {
        Some(SeverityNumber::Info2)
    } else if is("info") {
        Some(SeverityNumber::Info)
    } else if is("debug") {
        Some(SeverityNumber::Debug)
    } else {
        None
    }
}

pub(crate) struct CiseAttrs<'a> {
    /// The `KvList` for attribute `cise`.
    pub value: Value<'a>,
    pub time: Option<DateTime<Utc>>,
    pub severity: Option<SeverityNumber>,
}

fn pairs<'a>(pairs: Vec<(Cow<'a, str>, Cow<'a, str>)>) -> Value<'a> {
    Value::Array(
        pairs
            .into_iter()
            .map(|(k, v)| {
                Value::KvList(vec![
                    (ZString::from_static("key"), Value::String(ZString::Cow(k))),
                    (
                        ZString::from_static("value"),
                        Value::String(ZString::Cow(v)),
                    ),
                ])
            })
            .collect(),
    )
}

impl<'a> Cise<'a> {
    /// `received` is how many of the event's segments this record holds.
    pub(crate) fn into_attrs(
        self,
        category: &'a str,
        seg: &Segment<'_>,
        received: u32,
    ) -> CiseAttrs<'a> {
        let str = |k: &'static str, v: Cow<'a, str>| {
            (ZString::from_static(k), Value::String(ZString::Cow(v)))
        };
        let int = |k: &'static str, v: i64| (ZString::from_static(k), Value::Int64(v));
        let mut kv = Vec::with_capacity(11);
        kv.push(str("category", Cow::Borrowed(category)));
        kv.push(int("msg_id", seg.msg_id as i64));
        kv.push(int("total_seg", seg.total as i64));
        kv.push(int("segments", received as i64));
        if let Some(n) = self.sequence_num {
            kv.push(int("sequence_num", n));
        }
        if let Some(n) = self.code {
            kv.push(int("code", n));
        }
        if let Some(s) = self.severity {
            kv.push(str("severity", Cow::Borrowed(s)));
        }
        if let Some(s) = self.class {
            kv.push(str("class", Cow::Borrowed(s)));
        }
        if let Some(s) = self.text {
            kv.push(str("text", s));
        }
        if !self.attrs.is_empty() {
            kv.push((ZString::from_static("attrs"), pairs(self.attrs)));
        }
        if !self.response.is_empty() {
            kv.push((ZString::from_static("response"), pairs(self.response)));
        }
        CiseAttrs {
            value: Value::KvList(kv),
            time: self.time,
            severity: self.severity.and_then(severity_number),
        }
    }
}

/// Source address, ISE node (the syslog hostname) and `msg_id`: msg_ids count per node, and a
/// relay can forward several nodes from one address.
pub(crate) type Key = (IpAddr, SmolStr, u64);

/// An event put back together, or as much of it as arrived.
pub(crate) struct Assembled {
    pub addr: SocketAddr,
    /// The first segment held, whole, then the bodies of the others in order. It parses as a
    /// syslog message carrying that segment's counters.
    pub text: String,
    pub received: u32,
}

struct Part {
    /// The packet without its line ending.
    packet: String,
    body_at: usize,
}

struct Pending {
    first_seen: Instant,
    addr: SocketAddr,
    /// By `seg_num`.
    parts: Vec<Option<Part>>,
    received: u32,
    bytes: usize,
}

impl Pending {
    fn assemble(self) -> Assembled {
        let mut parts = self.parts.into_iter().flatten();
        let mut text = parts.next().map(|p| p.packet).unwrap_or_default();
        for p in parts {
            text.push_str(&p.packet[p.body_at..]);
        }
        Assembled {
            addr: self.addr,
            text,
            received: self.received,
        }
    }
}

/// Collects the segments of multi-segment events. Each segment is copied, so a held event pins
/// no receive buffer. Callers keep events of more than `max_segments` segments away, so a bogus
/// `total_seg` cannot make it allocate for them.
pub(crate) struct Reassembler {
    pending: FxHashMap<Key, Pending>,
    /// Start order, for expiry and eviction. An entry whose event is gone, or was started again,
    /// is skipped when it reaches the front.
    order: VecDeque<(Instant, Key)>,
    bytes: usize,
    timeout: Duration,
    max_pending: usize,
    max_pending_bytes: usize,
}

impl Reassembler {
    pub(crate) fn new(conf: &config::listener::Cise) -> Self {
        Self {
            pending: FxHashMap::default(),
            order: VecDeque::new(),
            bytes: 0,
            timeout: conf.reassembly_timeout,
            max_pending: conf.max_pending,
            max_pending_bytes: conf.max_pending_bytes,
        }
    }

    /// Takes one segment of `packet`. An event it completes, and any given up on to make room,
    /// go to `out`.
    pub(crate) fn offer(
        &mut self,
        key: Key,
        addr: SocketAddr,
        seg: &Segment<'_>,
        packet: &str,
        now: Instant,
        out: &mut Vec<Assembled>,
    ) {
        let body_at = seg.body.as_ptr() as usize - packet.as_ptr() as usize;
        let packet = &packet[..body_at + seg.body.len()];
        // The msg_id came back with another shape (ISE restarted): the old event is all in.
        if self
            .pending
            .get(&key)
            .is_some_and(|p| p.parts.len() != seg.total as usize)
            && let Some(old) = self.remove(&key)
        {
            out.push(old.assemble());
        }
        let order = &mut self.order;
        let entry = self.pending.entry(key.clone()).or_insert_with_key(|key| {
            order.push_back((now, key.clone()));
            Pending {
                first_seen: now,
                addr,
                parts: (0..seg.total).map(|_| None).collect(),
                received: 0,
                bytes: 0,
            }
        });
        let slot = &mut entry.parts[seg.num as usize];
        if slot.is_some() {
            return;
        }
        *slot = Some(Part {
            packet: packet.to_owned(),
            body_at,
        });
        entry.received += 1;
        entry.bytes += packet.len();
        self.bytes += packet.len();
        if entry.received == seg.total {
            if let Some(done) = self.remove(&key) {
                out.push(done.assemble());
            }
            return;
        }
        while self.pending.len() > self.max_pending || self.bytes > self.max_pending_bytes {
            let Some(oldest) = self.pop_oldest() else {
                break;
            };
            out.push(oldest.assemble());
        }
    }

    /// Gives up on the events whose first segment came the reassembly timeout before `now`.
    pub(crate) fn expire(&mut self, now: Instant, out: &mut Vec<Assembled>) {
        if self.pending.is_empty() {
            self.order.clear();
            return;
        }
        while let Some(&(seen, _)) = self.order.front() {
            if now.saturating_duration_since(seen) < self.timeout {
                break;
            }
            let (seen, key) = self.order.pop_front().expect("front exists");
            if self.is_live(seen, &key)
                && let Some(p) = self.remove(&key)
            {
                out.push(p.assemble());
            }
        }
    }

    /// Gives up on every event, oldest first.
    pub(crate) fn drain(&mut self, out: &mut Vec<Assembled>) {
        self.order.clear();
        self.bytes = 0;
        let mut all: Vec<Pending> = self.pending.drain().map(|(_, p)| p).collect();
        all.sort_unstable_by_key(|p| p.first_seen);
        out.extend(all.into_iter().map(Pending::assemble));
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    fn is_live(&self, seen: Instant, key: &Key) -> bool {
        self.pending.get(key).is_some_and(|p| p.first_seen == seen)
    }

    fn pop_oldest(&mut self) -> Option<Pending> {
        while let Some((seen, key)) = self.order.pop_front() {
            if self.is_live(seen, &key) {
                return self.remove(&key);
            }
        }
        None
    }

    fn remove(&mut self, key: &Key) -> Option<Pending> {
        let p = self.pending.remove(key)?;
        self.bytes -= p.bytes;
        Some(p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Made-up event: documentation addresses (RFC 5737), invented names.
    const SEG0: &str = "0000004711 2 0 2026-01-12 03:04:05.678 +07:00 0000000042 3300 NOTICE Tacacs-Accounting: TACACS+ Accounting with Command, ConfigVersionId=5, Device IP Address=192.0.2.10, CmdSet=[ CmdAV=show interfaces brief<cr> ], NetworkDeviceName=lab-switch-01, AVPair=task_id=7, AVPair=timezone = UTC, Step=13006, Step=15049, NetworkDeviceGroups=Location#All Locations#Lab, CPMSessionID=c0a8000a00000001,";
    const SEG1: &str = "0000004711 2 1  TotalAuthenLatency=3, ClientLatency=1, Network Device Profile=Cisco, Response={AcctReply-Status=Success; },";

    fn pairs_of<'a>(v: &'a [(Cow<'a, str>, Cow<'a, str>)]) -> Vec<(&'a str, &'a str)> {
        v.iter().map(|(k, v)| (k.as_ref(), v.as_ref())).collect()
    }

    #[test]
    fn categories() {
        assert_eq!(
            category("CISE_TACACS_Accounting"),
            Some("TACACS_Accounting")
        );
        assert_eq!(
            category("CSCOacs_Passed_Authentications"),
            Some("Passed_Authentications")
        );
        assert_eq!(category("CISE_"), None);
        assert_eq!(category("sshd"), None);
    }

    #[test]
    fn segment_counters() {
        let s = segment(SEG0).unwrap();
        assert_eq!((s.msg_id, s.total, s.num), (4711, 2, 0));
        assert!(s.body.starts_with("2026-01-12 "));
        assert_eq!(s.record, SEG0);

        let framed = format!(" {SEG1}\r\n");
        let s = segment(&framed).unwrap();
        assert_eq!((s.total, s.num), (2, 1));
        assert!(s.body.starts_with(" TotalAuthenLatency=3,"));
        assert!(s.record.ends_with("},"));

        let s = segment("7 1 0").unwrap();
        assert_eq!((s.msg_id, s.body), (7, ""));

        assert!(segment("7 2 2 x").is_none(), "seg_num past total");
        assert!(segment("7 0 0 x").is_none());
        assert!(segment("hello world").is_none());
        assert!(segment("7 2 x").is_none());
        assert!(segment("7 2 1x").is_none());
        assert!(segment("7 99999999999 0 x").is_none());
    }

    #[test]
    fn header_and_attributes() {
        let c = parse(segment(SEG0).unwrap().body);
        assert_eq!(
            c.time.unwrap().to_rfc3339(),
            "2026-01-11T20:04:05.678+00:00"
        );
        assert_eq!(c.sequence_num, Some(42));
        assert_eq!(c.code, Some(3300));
        assert_eq!(c.severity, Some("NOTICE"));
        assert_eq!(c.class, Some("Tacacs-Accounting"));
        assert_eq!(c.text.as_deref(), Some("TACACS+ Accounting with Command"));
        assert_eq!(
            pairs_of(&c.attrs),
            vec![
                ("ConfigVersionId", "5"),
                ("Device IP Address", "192.0.2.10"),
                ("CmdSet", "[ CmdAV=show interfaces brief<cr> ]"),
                ("NetworkDeviceName", "lab-switch-01"),
                ("AVPair", "task_id=7"),
                ("AVPair", "timezone = UTC"),
                ("Step", "13006"),
                ("Step", "15049"),
                ("NetworkDeviceGroups", "Location#All Locations#Lab"),
                ("CPMSessionID", "c0a8000a00000001"),
            ]
        );
        assert!(c.response.is_empty());
    }

    #[test]
    fn continuation_segment() {
        let c = parse(segment(SEG1).unwrap().body);
        assert!(c.time.is_none() && c.class.is_none() && c.text.is_none());
        assert_eq!(
            pairs_of(&c.attrs),
            vec![
                ("TotalAuthenLatency", "3"),
                ("ClientLatency", "1"),
                ("Network Device Profile", "Cisco"),
            ]
        );
        assert_eq!(pairs_of(&c.response), vec![("AcctReply-Status", "Success")]);
    }

    #[test]
    fn values_with_separators() {
        let c = parse(
            r"EAP-Key-Name=, cisco-av-pair=service-type=Framed, StepData=4= Radius.Called-Station-ID, DN=CN=a\,OU=b, Groups=x, y, Response={cisco-av-pair=url-redirect=https://portal.example.com/?a=b&c=d; Class=CACS:c0a8000a00000001; LicenseTypes=1; },",
        );
        assert_eq!(
            pairs_of(&c.attrs),
            vec![
                ("EAP-Key-Name", ""),
                ("cisco-av-pair", "service-type=Framed"),
                ("StepData", "4= Radius.Called-Station-ID"),
                ("DN", "CN=a,OU=b"),
                // An unescaped comma not followed by a pair stays in the value.
                ("Groups", "x, y"),
            ]
        );
        assert_eq!(
            pairs_of(&c.response),
            vec![
                (
                    "cisco-av-pair",
                    "url-redirect=https://portal.example.com/?a=b&c=d"
                ),
                ("Class", "CACS:c0a8000a00000001"),
                ("LicenseTypes", "1"),
            ]
        );
    }

    #[test]
    fn split_skips_escaped_and_bracketed_commas() {
        assert_eq!(
            split(r"A={x,[y,z]},B=[1,{2,3}],C=\,d,E=f,G=h\"),
            vec![r"A={x,[y,z]}", "B=[1,{2,3}]", r"C=\,d", "E=f", r"G=h\"]
        );
        assert_eq!(split(""), vec![""]);
        assert_eq!(split("no separators"), vec!["no separators"]);
    }

    #[test]
    fn header_without_code() {
        let c = parse(
            "2026-01-12 03:04:05.678 -05:30 0000012345 WARN System-Management: ISE disk is almost full, Server=ise-lab-02,",
        );
        assert_eq!(
            c.time.unwrap().to_rfc3339(),
            "2026-01-12T08:34:05.678+00:00"
        );
        assert_eq!(c.sequence_num, Some(12345));
        assert_eq!(c.code, None);
        assert_eq!(c.severity, Some("WARN"));
        assert_eq!(c.class, Some("System-Management"));
        assert_eq!(c.text.as_deref(), Some("ISE disk is almost full"));
        assert_eq!(pairs_of(&c.attrs), vec![("Server", "ise-lab-02")]);
    }

    #[test]
    fn unparsable_time_keeps_the_rest() {
        let c = parse("2026-01-12 03:04:05 XYZ 1 5200 INFO Cls: text, a=b,");
        assert!(c.time.is_none());
        assert_eq!(c.code, Some(5200));
        assert_eq!(c.class, Some("Cls"));
        assert_eq!(pairs_of(&c.attrs), vec![("a", "b")]);
    }

    #[test]
    fn severities() {
        assert_eq!(severity_number("NOTICE"), Some(SeverityNumber::Info2));
        assert_eq!(severity_number("INFO"), Some(SeverityNumber::Info));
        assert_eq!(severity_number("WARN"), Some(SeverityNumber::Warn));
        assert_eq!(severity_number("ERROR"), Some(SeverityNumber::Error));
        assert_eq!(severity_number("FATAL"), Some(SeverityNumber::Fatal));
        assert_eq!(severity_number("DEBUG"), Some(SeverityNumber::Debug));
        assert_eq!(severity_number("OTHER"), None);
    }

    #[test]
    fn attrs_layout() {
        let seg = segment(SEG1).unwrap();
        let a = parse(seg.body).into_attrs("TACACS_Accounting", &seg, 1);
        let Value::KvList(kv) = &a.value else {
            panic!("not a kvlist");
        };
        let keys: Vec<_> = kv.iter().map(|(k, _)| k.to_string()).collect();
        assert_eq!(
            keys,
            [
                "category",
                "msg_id",
                "total_seg",
                "segments",
                "attrs",
                "response"
            ]
        );
        let Value::Array(attrs) = &kv[4].1 else {
            panic!("attrs not an array");
        };
        let Value::KvList(first) = &attrs[2] else {
            panic!("pair not a kvlist");
        };
        assert!(
            matches!(&first[0], (k, Value::String(v)) if k.to_string() == "key" && v.to_string() == "Network Device Profile")
        );
        assert!(
            matches!(&first[1], (k, Value::String(v)) if k.to_string() == "value" && v.to_string() == "Cisco")
        );
        assert!(a.severity.is_none() && a.time.is_none());
    }

    // ----- Reassembler -----

    const TIMEOUT: Duration = Duration::from_secs(10);

    fn addr(last: u8) -> SocketAddr {
        SocketAddr::from(([192, 0, 2, last], 514))
    }

    fn key(last: u8, host: &str, msg_id: u64) -> Key {
        (addr(last).ip(), SmolStr::new(host), msg_id)
    }

    fn packet(msg_id: u64, total: u32, num: u32, body: &str) -> String {
        format!("<181>Jan 12 03:04:05 ise-lab-01 CISE_Test {msg_id} {total} {num} {body}\n")
    }

    /// Offers `packet` as it would arrive: the segment is found after the tag.
    fn offer(r: &mut Reassembler, k: Key, packet: &str, now: Instant) -> Vec<Assembled> {
        let after_tag = packet.find("CISE_Test").unwrap() + "CISE_Test".len();
        let seg = segment(&packet[after_tag..]).unwrap();
        let mut out = Vec::new();
        r.offer(k, addr(1), &seg, packet, now, &mut out);
        out
    }

    #[test]
    fn completes_out_of_order() {
        let mut r = Reassembler::new(&config::listener::Cise::default());
        let now = Instant::now();
        let (p0, p1, p2) = (
            packet(
                5,
                3,
                0,
                "2026-01-12 03:04:05.678 +07:00 1 5200 NOTICE C: t, a=1,",
            ),
            packet(5, 3, 1, " b=2,"),
            packet(5, 3, 2, " c=3,"),
        );
        assert!(offer(&mut r, key(1, "ise-lab-01", 5), &p2, now).is_empty());
        assert!(offer(&mut r, key(1, "ise-lab-01", 5), &p0, now).is_empty());
        // A duplicate is ignored.
        assert!(offer(&mut r, key(1, "ise-lab-01", 5), &p2, now).is_empty());
        let out = offer(&mut r, key(1, "ise-lab-01", 5), &p1, now);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].received, 3);
        assert_eq!(
            out[0].text,
            "<181>Jan 12 03:04:05 ise-lab-01 CISE_Test 5 3 0 2026-01-12 03:04:05.678 +07:00 1 5200 NOTICE C: t, a=1, b=2, c=3,"
        );
        assert!(r.is_empty());
        assert_eq!(r.bytes, 0);
    }

    #[test]
    fn keys_stay_apart() {
        let mut r = Reassembler::new(&config::listener::Cise::default());
        let now = Instant::now();
        let p = packet(5, 2, 0, "x=1,");
        offer(&mut r, key(1, "ise-lab-01", 5), &p, now);
        offer(&mut r, key(1, "ise-lab-02", 5), &p, now);
        offer(&mut r, key(2, "ise-lab-01", 5), &p, now);
        assert_eq!(r.pending.len(), 3);
    }

    #[test]
    fn expires_incomplete() {
        let mut r = Reassembler::new(&config::listener::Cise::default());
        let t0 = Instant::now();
        offer(
            &mut r,
            key(1, "ise-lab-01", 1),
            &packet(1, 3, 1, " b=2,"),
            t0,
        );
        offer(
            &mut r,
            key(1, "ise-lab-01", 1),
            &packet(1, 3, 2, " c=3,"),
            t0,
        );
        offer(
            &mut r,
            key(1, "ise-lab-01", 2),
            &packet(2, 2, 0, "a=1,"),
            t0 + Duration::from_secs(5),
        );
        let mut out = Vec::new();
        r.expire(t0 + TIMEOUT - Duration::from_millis(1), &mut out);
        assert!(out.is_empty());
        r.expire(t0 + TIMEOUT, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].received, 2);
        // Segment 0 never came: the first held segment leads.
        assert_eq!(
            out[0].text,
            "<181>Jan 12 03:04:05 ise-lab-01 CISE_Test 1 3 1  b=2, c=3,"
        );
        assert_eq!(r.pending.len(), 1);

        out.clear();
        r.drain(&mut out);
        assert_eq!(out.len(), 1);
        assert!(r.is_empty() && r.order.is_empty());
    }

    #[test]
    fn completed_events_leave_no_expiry() {
        let mut r = Reassembler::new(&config::listener::Cise::default());
        let t0 = Instant::now();
        offer(
            &mut r,
            key(1, "ise-lab-01", 1),
            &packet(1, 2, 0, "a=1,"),
            t0,
        );
        offer(
            &mut r,
            key(1, "ise-lab-01", 1),
            &packet(1, 2, 1, " b=2,"),
            t0,
        );
        // Started again under the same key after the first one completed.
        let t1 = t0 + Duration::from_secs(8);
        offer(
            &mut r,
            key(1, "ise-lab-01", 1),
            &packet(1, 2, 0, "a=3,"),
            t1,
        );
        let mut out = Vec::new();
        // The first event's queue entry is stale and must not expire the second.
        r.expire(t0 + TIMEOUT, &mut out);
        assert!(out.is_empty());
        assert_eq!(r.pending.len(), 1);
        r.expire(t1 + TIMEOUT, &mut out);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn reshaped_msg_id_flushes_the_old_event() {
        let mut r = Reassembler::new(&config::listener::Cise::default());
        let now = Instant::now();
        offer(
            &mut r,
            key(1, "ise-lab-01", 1),
            &packet(1, 3, 0, "a=1,"),
            now,
        );
        let out = offer(
            &mut r,
            key(1, "ise-lab-01", 1),
            &packet(1, 2, 0, "a=2,"),
            now,
        );
        assert_eq!(out.len(), 1);
        assert!(out[0].text.ends_with("1 3 0 a=1,"));
        assert_eq!(r.pending.len(), 1);
    }

    #[test]
    fn evicts_oldest_past_max_pending() {
        let mut r = Reassembler::new(&config::listener::Cise {
            max_pending: 4,
            ..Default::default()
        });
        let now = Instant::now();
        let mut out = Vec::new();
        for id in 0..6 {
            out.extend(offer(
                &mut r,
                key(1, "ise-lab-01", id),
                &packet(id, 2, 0, "a=1,"),
                now,
            ));
        }
        assert_eq!(out.len(), 2);
        assert!(out[0].text.contains("CISE_Test 0 2 0 "));
        assert!(out[1].text.contains("CISE_Test 1 2 0 "));
        assert_eq!(r.pending.len(), 4);
    }

    #[test]
    fn evicts_oldest_past_max_pending_bytes() {
        let one = packet(0, 2, 0, "a=1,").trim_end().len();
        let mut r = Reassembler::new(&config::listener::Cise {
            max_pending_bytes: one * 2,
            ..Default::default()
        });
        let now = Instant::now();
        let mut out = Vec::new();
        for id in 0..3 {
            out.extend(offer(
                &mut r,
                key(1, "ise-lab-01", id),
                &packet(id, 2, 0, "a=1,"),
                now,
            ));
        }
        assert_eq!(out.len(), 1);
        assert!(out[0].text.contains("CISE_Test 0 2 0 "));
        assert_eq!(r.bytes, one * 2);
    }
}

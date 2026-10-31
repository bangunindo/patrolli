// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Common Event Format (CEF) parser.
//!
//! `CEF:Version|Device Vendor|Device Product|Device Version|Device Event Class ID|Name|Severity|Extension`
//!
//! Parsing is split in two: [`parse`] cuts the record into header fields and raw extension pairs,
//! borrowing from the input unless a field carries escapes; [`Cef::into_attrs`] types the values and
//! builds the `cef` attribute (`cef.<header>`, `cef.ext.<key>`, `cef.ext.custom.N.{key,label,value}`).

use super::time::parse_time;
use crate::payload::{Value, ZString};
use chrono::{DateTime, Utc};
use opentelemetry_proto::tonic::logs::v1::SeverityNumber;
use std::borrow::Cow;

const PREFIX: &str = "CEF:";
const HEADER_FIELDS: usize = 7;

pub(crate) struct Cef<'a> {
    pub version: &'a str,
    pub device_vendor: Cow<'a, str>,
    pub device_product: Cow<'a, str>,
    pub device_version: Cow<'a, str>,
    pub device_event_class_id: Cow<'a, str>,
    pub name: Cow<'a, str>,
    pub severity: Cow<'a, str>,
    /// Raw (unsanitized) key and unescaped value, in arrival order.
    pub extensions: Vec<(&'a str, Cow<'a, str>)>,
}

#[derive(thiserror::Error, Debug, PartialEq, Eq)]
pub(crate) enum CefError {
    #[error("missing CEF: prefix")]
    MissingPrefix,
    #[error("invalid CEF version")]
    InvalidVersion,
    #[error("CEF header has fewer than {HEADER_FIELDS} fields")]
    TruncatedHeader,
}

/// Byte offset of the first `CEF:<digit>` in `packet`.
pub(crate) fn find(packet: &str) -> Option<usize> {
    let bytes = packet.as_bytes();
    let mut from = 0;
    while let Some(pos) = packet[from..].find(PREFIX) {
        let at = from + pos;
        if bytes
            .get(at + PREFIX.len())
            .is_some_and(|b| b.is_ascii_digit())
        {
            return Some(at);
        }
        from = at + PREFIX.len();
    }
    None
}

/// Parses a CEF record. `input` must start at `CEF:`.
pub(crate) fn parse(input: &str) -> Result<Cef<'_>, CefError> {
    let body = input.strip_prefix(PREFIX).ok_or(CefError::MissingPrefix)?;
    let bytes = body.as_bytes();
    let mut fields: [&str; HEADER_FIELDS] = [""; HEADER_FIELDS];
    let mut count = 0;
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() && count < HEADER_FIELDS {
        match bytes[i] {
            b'\\' => i += 2,
            b'|' => {
                fields[count] = &body[start..i];
                count += 1;
                i += 1;
                start = i;
            }
            _ => i += 1,
        }
    }
    if count < HEADER_FIELDS {
        return Err(CefError::TruncatedHeader);
    }
    let [
        version,
        vendor,
        product,
        dev_version,
        class_id,
        name,
        severity,
    ] = fields;
    if version.is_empty() || !version.bytes().all(|b| b.is_ascii_digit()) {
        return Err(CefError::InvalidVersion);
    }
    Ok(Cef {
        version,
        device_vendor: unescape_header(vendor),
        device_product: unescape_header(product),
        device_version: unescape_header(dev_version),
        device_event_class_id: unescape_header(class_id),
        name: unescape_header(name),
        severity: unescape_header(severity),
        extensions: parse_extension(&body[start..]),
    })
}

fn unescape_header(s: &str) -> Cow<'_, str> {
    if !s.contains('\\') {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some(e @ ('|' | '\\')) => out.push(e),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    Cow::Owned(out)
}

fn unescape_value(s: &str) -> Cow<'_, str> {
    if !s.contains('\\') {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some(e @ ('=' | '\\')) => out.push(e),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    Cow::Owned(out)
}

/// Splits `k1=v1 k2=v two k3=v3` into pairs. An unescaped `=` is a key boundary only when a space
/// separates it from the previous boundary, so `request=http://h/?a=b` keeps `a=b` in the value.
fn parse_extension(ext: &str) -> Vec<(&str, Cow<'_, str>)> {
    let bytes = ext.as_bytes();
    let mut out = Vec::new();
    // Key of the pair being read and where its value starts.
    let mut current: Option<(&str, usize)> = None;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => {
                i += 2;
                continue;
            }
            b'=' => {
                let region_start = current.map_or(0, |(_, value_start)| value_start);
                let region = &ext[region_start..i];
                let key_start = region.rfind(' ').map(|sp| sp + 1);
                match (current, key_start) {
                    (None, _) => {
                        let key = &region[key_start.unwrap_or(0)..];
                        if !key.is_empty() {
                            current = Some((key, i + 1));
                        }
                    }
                    (Some((key, value_start)), Some(ks)) if ks < region.len() => {
                        let value = ext[value_start..region_start + ks].trim_end();
                        out.push((key, unescape_value(value)));
                        current = Some((&region[ks..], i + 1));
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        i += 1;
    }
    if let Some((key, value_start)) = current {
        let value = ext.get(value_start..).unwrap_or("").trim_end();
        out.push((key, unescape_value(value)));
    }
    out
}

pub(crate) struct CefAttrs<'a> {
    /// The `KvList` for attribute `cef`.
    pub value: Value<'a>,
    /// Parsed `rt` (deviceReceiptTime), for the record's observed time.
    pub rt: Option<DateTime<Utc>>,
    pub severity: Option<SeverityNumber>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum KeyType {
    Int,
    Float,
    Time,
}

/// Non-string types from the CEF key dictionary. Everything else stays a string.
fn key_type(raw: &str) -> Option<KeyType> {
    Some(match raw {
        "cnt"
        | "dpid"
        | "dpt"
        | "dvcpid"
        | "fsize"
        | "in"
        | "out"
        | "oldFileSize"
        | "spid"
        | "spt"
        | "sourceTranslatedPort"
        | "destinationTranslatedPort"
        | "type"
        | "deviceDirection"
        | "eventId" => KeyType::Int,
        "slat" | "slong" | "dlat" | "dlong" => KeyType::Float,
        "rt"
        | "start"
        | "end"
        | "art"
        | "fileCreateTime"
        | "fileModificationTime"
        | "oldFileCreateTime"
        | "oldFileModificationTime" => KeyType::Time,
        _ => return None,
    })
}

/// Custom slot kinds, in the order they are listed in `cef.ext.custom`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum SlotKind {
    Cs,
    Cn,
    Cfp,
    C6a,
    DeviceCustomDate,
    FlexString,
    FlexNumber,
    FlexDate,
}

const SLOT_PREFIXES: [(&str, SlotKind); 8] = [
    ("cs", SlotKind::Cs),
    ("cn", SlotKind::Cn),
    ("cfp", SlotKind::Cfp),
    ("c6a", SlotKind::C6a),
    ("deviceCustomDate", SlotKind::DeviceCustomDate),
    ("flexString", SlotKind::FlexString),
    ("flexNumber", SlotKind::FlexNumber),
    ("flexDate", SlotKind::FlexDate),
];

impl SlotKind {
    fn key_type(self) -> Option<KeyType> {
        match self {
            SlotKind::Cn | SlotKind::FlexNumber => Some(KeyType::Int),
            SlotKind::Cfp => Some(KeyType::Float),
            SlotKind::DeviceCustomDate | SlotKind::FlexDate => Some(KeyType::Time),
            SlotKind::Cs | SlotKind::C6a | SlotKind::FlexString => None,
        }
    }
}

/// `cs1` → `(Cs, 1, false)`, `cs1Label` → `(Cs, 1, true)`.
fn slot(raw: &str) -> Option<(SlotKind, u32, bool)> {
    if !matches!(raw.as_bytes().first(), Some(b'c' | b'd' | b'f')) {
        return None;
    }
    SLOT_PREFIXES.iter().find_map(|&(prefix, kind)| {
        let rest = raw.strip_prefix(prefix)?;
        let (digits, is_label) = match rest.strip_suffix("Label") {
            Some(d) => (d, true),
            None => (rest, false),
        };
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        Some((kind, digits.parse().ok()?, is_label))
    })
}

struct Slot<'a> {
    kind: SlotKind,
    idx: u32,
    label: Option<Cow<'a, str>>,
    /// Raw slot key (`cs1`) and its value.
    value: Option<(&'a str, Cow<'a, str>)>,
}

fn sanitize_key(raw: &str) -> ZString<'_> {
    if raw.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return ZString::from_ref(raw);
    }
    ZString::from_owned(
        raw.chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect(),
    )
}

/// Casts to the key's native type, falling back to the string. A `Time` value also returns the
/// parsed instant.
fn cast(ty: Option<KeyType>, v: Cow<'_, str>) -> (Value<'_>, Option<DateTime<Utc>>) {
    match ty {
        Some(KeyType::Int) => {
            if let Ok(n) = v.parse::<i64>() {
                return (Value::Int64(n), None);
            }
        }
        Some(KeyType::Float) => {
            if let Ok(f) = v.parse::<f64>()
                && f.is_finite()
            {
                return (Value::Double(f), None);
            }
        }
        Some(KeyType::Time) => {
            if let Some(t) = parse_time(&v) {
                return (Value::Int64(t.timestamp_millis()), Some(t));
            }
        }
        None => {}
    }
    (Value::String(ZString::Cow(v)), None)
}

/// CEF severity (0–10 or Unknown/Low/Medium/High/Very-High) to the OpenTelemetry scale.
pub(crate) fn severity_number(sev: &str) -> Option<SeverityNumber> {
    let sev = sev.trim();
    if let Ok(n) = sev.parse::<u8>() {
        return match n {
            0..=3 => Some(SeverityNumber::Info),
            4..=6 => Some(SeverityNumber::Warn),
            7..=8 => Some(SeverityNumber::Error),
            9..=10 => Some(SeverityNumber::Fatal),
            _ => None,
        };
    }
    let is = |name: &str| sev.eq_ignore_ascii_case(name);
    if is("low") {
        Some(SeverityNumber::Info)
    } else if is("medium") {
        Some(SeverityNumber::Warn)
    } else if is("high") {
        Some(SeverityNumber::Error)
    } else if is("very-high") || is("very high") {
        Some(SeverityNumber::Fatal)
    } else {
        None
    }
}

impl<'a> Cef<'a> {
    pub(crate) fn into_attrs(self) -> CefAttrs<'a> {
        let severity = severity_number(&self.severity);
        let mut rt = None;
        let mut ext: Vec<(ZString<'a>, Value<'a>)> = Vec::with_capacity(self.extensions.len());
        let mut slots: Vec<Slot<'a>> = Vec::new();

        for (raw, value) in self.extensions {
            if let Some((kind, idx, is_label)) = slot(raw) {
                let pos = match slots.iter().position(|s| s.kind == kind && s.idx == idx) {
                    Some(pos) => pos,
                    None => {
                        slots.push(Slot {
                            kind,
                            idx,
                            label: None,
                            value: None,
                        });
                        slots.len() - 1
                    }
                };
                if is_label {
                    slots[pos].label = Some(value);
                } else {
                    slots[pos].value = Some((raw, value));
                }
                continue;
            }
            let (value, time) = cast(key_type(raw), value);
            if raw == "rt" && time.is_some() {
                rt = time;
            }
            ext.push((sanitize_key(raw), value));
        }

        slots.sort_unstable_by_key(|s| (s.kind, s.idx));
        let custom: Vec<Value<'a>> = slots
            .into_iter()
            .filter_map(|s| {
                let (key, value) = s.value?;
                let mut kv = Vec::with_capacity(3);
                kv.push((
                    ZString::from_static("key"),
                    Value::String(ZString::from_ref(key)),
                ));
                if let Some(label) = s.label {
                    kv.push((
                        ZString::from_static("label"),
                        Value::String(ZString::Cow(label)),
                    ));
                }
                kv.push((
                    ZString::from_static("value"),
                    cast(s.kind.key_type(), value).0,
                ));
                Some(Value::KvList(kv))
            })
            .collect();
        if !custom.is_empty() {
            ext.push((ZString::from_static("custom"), Value::Array(custom)));
        }

        let header = |k: &'static str, v: Cow<'a, str>| {
            (ZString::from_static(k), Value::String(ZString::Cow(v)))
        };
        let mut cef = Vec::with_capacity(HEADER_FIELDS + 1);
        cef.push(header("version", Cow::Borrowed(self.version)));
        cef.push(header("device_vendor", self.device_vendor));
        cef.push(header("device_product", self.device_product));
        cef.push(header("device_version", self.device_version));
        cef.push(header("device_event_class_id", self.device_event_class_id));
        cef.push(header("name", self.name));
        cef.push(header("severity", self.severity));
        if !ext.is_empty() {
            cef.push((ZString::from_static("ext"), Value::KvList(ext)));
        }

        CefAttrs {
            value: Value::KvList(cef),
            rt,
            severity,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq)]
    enum Flat {
        Str(String),
        Int(i64),
        Dbl(f64),
    }

    /// Flattens the way `Value::append_to` lays attributes out.
    fn flatten(prefix: &str, v: &Value, out: &mut Vec<(String, Flat)>) {
        match v {
            Value::String(s) => out.push((prefix.to_string(), Flat::Str(s.to_string()))),
            Value::Int64(n) => out.push((prefix.to_string(), Flat::Int(*n))),
            Value::Double(f) => out.push((prefix.to_string(), Flat::Dbl(*f))),
            Value::Array(a) => {
                for (i, v) in a.iter().enumerate() {
                    flatten(&format!("{prefix}.{i}"), v, out);
                }
            }
            Value::KvList(l) => {
                for (k, v) in l {
                    flatten(&format!("{prefix}.{k}"), v, out);
                }
            }
            _ => panic!("unexpected value type"),
        }
    }

    fn attrs(input: &str) -> (Vec<(String, Flat)>, CefAttrs<'_>) {
        let a = parse(input).unwrap().into_attrs();
        let mut out = Vec::new();
        flatten("cef", &a.value, &mut out);
        (out, a)
    }

    fn get<'v>(flat: &'v [(String, Flat)], key: &str) -> Option<&'v Flat> {
        flat.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    fn s(v: &str) -> Flat {
        Flat::Str(v.to_string())
    }

    fn ext<'a>(c: &'a Cef) -> Vec<(&'a str, &'a str)> {
        c.extensions.iter().map(|(k, v)| (*k, v.as_ref())).collect()
    }

    #[test]
    fn spec_example() {
        let c = parse(
            "CEF:0|Security|threatmanager|1.0|100|worm successfully stopped|10|src=10.0.0.1 dst=2.1.2.2 spt=1232",
        )
        .unwrap();
        assert_eq!(c.version, "0");
        assert_eq!(c.device_vendor, "Security");
        assert_eq!(c.device_product, "threatmanager");
        assert_eq!(c.device_version, "1.0");
        assert_eq!(c.device_event_class_id, "100");
        assert_eq!(c.name, "worm successfully stopped");
        assert_eq!(c.severity, "10");
        assert_eq!(
            ext(&c),
            vec![("src", "10.0.0.1"), ("dst", "2.1.2.2"), ("spt", "1232")]
        );
    }

    #[test]
    fn header_escapes() {
        let c = parse(r"CEF:0|Ven\|dor|back\\slash|1|2|n|3|").unwrap();
        assert_eq!(c.device_vendor, "Ven|dor");
        assert_eq!(c.device_product, r"back\slash");
        assert!(matches!(c.device_version, Cow::Borrowed(_)));
        assert!(c.extensions.is_empty());
    }

    #[test]
    fn extension_values() {
        let c = parse(
            r"CEF:0|V|P|1|2|n|3|msg=hello world  foo=a\=b c=x\\y d=l1\nl2 request=http://h/?a=b&c=d e=pipe\|kept",
        )
        .unwrap();
        assert_eq!(
            ext(&c),
            vec![
                ("msg", "hello world"),
                ("foo", "a=b"),
                ("c", r"x\y"),
                ("d", "l1\nl2"),
                ("request", "http://h/?a=b&c=d"),
                ("e", r"pipe\|kept"),
            ]
        );
    }

    #[test]
    fn extension_trailing_whitespace_and_empty() {
        let c = parse("CEF:1|V|P|1|2|n|3|a=1 b=  ").unwrap();
        assert_eq!(c.version, "1");
        assert_eq!(ext(&c), vec![("a", "1"), ("b", "")]);
        assert!(
            parse("CEF:0|V|P|1|2|n|3|   ")
                .unwrap()
                .extensions
                .is_empty()
        );
    }

    #[test]
    fn errors() {
        assert_eq!(
            parse("CEF:0|V|P|1|2|n").err(),
            Some(CefError::TruncatedHeader)
        );
        assert_eq!(
            parse("CEF:x|V|P|1|2|n|3|").err(),
            Some(CefError::InvalidVersion)
        );
        assert_eq!(parse("0|V|P|1|2|n|3|").err(), Some(CefError::MissingPrefix));
    }

    #[test]
    fn find_offsets() {
        let l3164 = "<13>Sep 19 08:26:10 host CEF:0|V|P|1|2|n|3|";
        assert_eq!(find(l3164), Some(25));
        let l5424 = "<13>1 2026-09-19T08:26:10Z host app - - - CEF:0|V|P|1|2|n|3|";
        assert_eq!(&l5424[find(l5424).unwrap()..][..5], "CEF:0");
        assert_eq!(find("CEF:0|V|P|1|2|n|3|"), Some(0));
        assert_eq!(find("<13>Sep 19 08:26:10 host plain text"), None);
        assert_eq!(find("<13>note CEF: x then CEF:1|V"), Some(21));
        assert_eq!(find("trailing CEF:"), None);
    }

    #[test]
    fn sanitizes_keys() {
        let (flat, _) = attrs("CEF:0|V|P|1|2|n|3|ad.user[0]=bob src=1.2.3.4");
        assert_eq!(get(&flat, "cef.ext.ad_user_0_"), Some(&s("bob")));
        assert_eq!(get(&flat, "cef.ext.src"), Some(&s("1.2.3.4")));
        assert!(matches!(
            sanitize_key("src"),
            ZString::Cow(Cow::Borrowed(_))
        ));
    }

    #[test]
    fn header_attrs() {
        let (flat, a) = attrs("CEF:0|Security|tm|1.0|100|worm|10|");
        assert_eq!(get(&flat, "cef.version"), Some(&s("0")));
        assert_eq!(get(&flat, "cef.device_vendor"), Some(&s("Security")));
        assert_eq!(get(&flat, "cef.device_event_class_id"), Some(&s("100")));
        assert_eq!(get(&flat, "cef.severity"), Some(&s("10")));
        assert!(flat.iter().all(|(k, _)| !k.starts_with("cef.ext")));
        assert_eq!(a.severity, Some(SeverityNumber::Fatal));
    }

    #[test]
    fn typed_casts() {
        let (flat, _) = attrs("CEF:0|V|P|1|2|n|3|spt=1232 slat=1.5 dpt=abc cnt=7 src=10.0.0.1");
        assert_eq!(get(&flat, "cef.ext.spt"), Some(&Flat::Int(1232)));
        assert_eq!(get(&flat, "cef.ext.slat"), Some(&Flat::Dbl(1.5)));
        assert_eq!(get(&flat, "cef.ext.dpt"), Some(&s("abc")));
        assert_eq!(get(&flat, "cef.ext.cnt"), Some(&Flat::Int(7)));
        assert_eq!(get(&flat, "cef.ext.src"), Some(&s("10.0.0.1")));
    }

    #[test]
    fn rt_epoch_millis() {
        let (flat, a) = attrs("CEF:0|V|P|1|2|n|3|rt=1758270370123");
        assert_eq!(get(&flat, "cef.ext.rt"), Some(&Flat::Int(1758270370123)));
        assert_eq!(a.rt.unwrap().timestamp_millis(), 1758270370123);
    }

    #[test]
    fn rt_unknown_zone_stays_string() {
        let (flat, a) = attrs("CEF:0|V|P|1|2|n|3|rt=Sep 19 2026 08:26:10 WIB");
        assert_eq!(
            get(&flat, "cef.ext.rt"),
            Some(&s("Sep 19 2026 08:26:10 WIB"))
        );
        assert!(a.rt.is_none());
    }

    #[test]
    fn custom_slots() {
        let (flat, _) = attrs(
            "CEF:0|V|P|1|2|n|3|cn1=42 cs2=second cs1Label=Policy cs1=block-all \
             cfp1Label=Score cfp1=0.5 deviceCustomDate1=1758270370000 cn2Label=orphan cnt=3",
        );
        let custom: Vec<_> = flat
            .iter()
            .filter(|(k, _)| k.starts_with("cef.ext.custom"))
            .map(|(k, v)| (k.as_str(), v))
            .collect();
        assert_eq!(
            custom,
            vec![
                ("cef.ext.custom.0.key", &s("cs1")),
                ("cef.ext.custom.0.label", &s("Policy")),
                ("cef.ext.custom.0.value", &s("block-all")),
                ("cef.ext.custom.1.key", &s("cs2")),
                ("cef.ext.custom.1.value", &s("second")),
                ("cef.ext.custom.2.key", &s("cn1")),
                ("cef.ext.custom.2.value", &Flat::Int(42)),
                ("cef.ext.custom.3.key", &s("cfp1")),
                ("cef.ext.custom.3.label", &s("Score")),
                ("cef.ext.custom.3.value", &Flat::Dbl(0.5)),
                ("cef.ext.custom.4.key", &s("deviceCustomDate1")),
                ("cef.ext.custom.4.value", &Flat::Int(1758270370000)),
            ]
        );
        assert_eq!(get(&flat, "cef.ext.cnt"), Some(&Flat::Int(3)));
        assert!(get(&flat, "cef.ext.cs1").is_none());
        assert!(get(&flat, "cef.ext.cs1Label").is_none());
        assert!(get(&flat, "cef.ext.cn2Label").is_none());
    }

    #[test]
    fn slot_matching() {
        assert!(slot("cs1") == Some((SlotKind::Cs, 1, false)));
        assert!(slot("c6a4Label") == Some((SlotKind::C6a, 4, true)));
        assert!(slot("flexDate1") == Some((SlotKind::FlexDate, 1, false)));
        assert!(slot("cnt").is_none());
        assert!(slot("cs").is_none());
        assert!(slot("csLabel").is_none());
        assert!(slot("src").is_none());
    }

    #[test]
    fn severity_mapping() {
        assert_eq!(severity_number("3"), Some(SeverityNumber::Info));
        assert_eq!(severity_number("4"), Some(SeverityNumber::Warn));
        assert_eq!(severity_number("6"), Some(SeverityNumber::Warn));
        assert_eq!(severity_number("7"), Some(SeverityNumber::Error));
        assert_eq!(severity_number("8"), Some(SeverityNumber::Error));
        assert_eq!(severity_number("9"), Some(SeverityNumber::Fatal));
        assert_eq!(severity_number("11"), None);
        assert_eq!(severity_number("low"), Some(SeverityNumber::Info));
        assert_eq!(severity_number("Medium"), Some(SeverityNumber::Warn));
        assert_eq!(severity_number("HIGH"), Some(SeverityNumber::Error));
        assert_eq!(severity_number("Very-High"), Some(SeverityNumber::Fatal));
        assert_eq!(severity_number("Unknown"), None);
    }
}

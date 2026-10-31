// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Device timestamps as CEF and Cisco send them.

use chrono::{
    DateTime, Datelike, FixedOffset, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc,
};
use std::str::FromStr;

/// Accepts epoch milliseconds, `MMM dd [yyyy] HH:mm:ss[.SSS] [zone]` or (NX-OS)
/// `yyyy MMM dd HH:mm:ss[.SSS] [zone]`. A missing year is the
/// current one and a missing zone is local time, matching how syslog_loose reads RFC3164 dates.
pub(crate) fn parse_time(s: &str) -> Option<DateTime<Utc>> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if s.bytes().all(|b| b.is_ascii_digit()) {
        return DateTime::from_timestamp_millis(s.parse().ok()?);
    }
    if let Some(naive) = parse_naive(s) {
        return Local
            .from_local_datetime(&naive)
            .earliest()
            .map(|t| t.with_timezone(&Utc));
    }
    let (rest, zone) = s.rsplit_once(' ')?;
    resolve_zone(parse_naive(rest.trim_end())?, zone)
}

/// `MMM dd [yyyy] HH:mm:ss[.f]` or `yyyy MMM dd HH:mm:ss[.f]`, words separated by any run of
/// spaces (IOS pads a one-digit day). Hand-rolled: chrono's format-string parser was most of
/// the cost of a Cisco message.
fn parse_naive(s: &str) -> Option<NaiveDateTime> {
    let mut words = s.split_ascii_whitespace();
    let first = words.next()?;
    let (year, month, day, clock) = match number(first, 4, 4) {
        Some(year) => (
            Some(year),
            month(words.next()?)?,
            number(words.next()?, 1, 2)?,
            words.next()?,
        ),
        None => {
            let month = month(first)?;
            let day = number(words.next()?, 1, 2)?;
            let next = words.next()?;
            match number(next, 4, 4) {
                Some(year) => (Some(year), month, day, words.next()?),
                None => (None, month, day, next),
            }
        }
    };
    if words.next().is_some() {
        return None;
    }
    let time = clock_time(clock)?;
    let year = match year {
        Some(year) => year as i32,
        None => Local::now().year(),
    };
    Some(NaiveDate::from_ymd_opt(year, month, day)?.and_time(time))
}

/// `min` to `max` ASCII digits, nothing else.
fn number(s: &str, min: usize, max: usize) -> Option<u32> {
    if !(min..=max).contains(&s.len()) {
        return None;
    }
    s.bytes().try_fold(0u32, |n, b| {
        b.is_ascii_digit().then(|| n * 10 + u32::from(b - b'0'))
    })
}

/// `Jan` or `January`, any case. 1-based.
fn month(s: &str) -> Option<u32> {
    const MONTHS: [&str; 12] = [
        "january",
        "february",
        "march",
        "april",
        "may",
        "june",
        "july",
        "august",
        "september",
        "october",
        "november",
        "december",
    ];
    let i = MONTHS.iter().position(|m| {
        s.len() == 3 && m[..3].eq_ignore_ascii_case(s) || m.eq_ignore_ascii_case(s)
    })?;
    Some(i as u32 + 1)
}

/// `HH:mm:ss[.f]`, up to nine fraction digits. A second of 60 is a leap second.
fn clock_time(s: &str) -> Option<NaiveTime> {
    let (hms, frac) = match s.split_once('.') {
        Some((hms, frac)) => (hms, Some(frac)),
        None => (s, None),
    };
    let mut parts = hms.split(':');
    let h = number(parts.next()?, 1, 2)?;
    let m = number(parts.next()?, 1, 2)?;
    let sec = number(parts.next()?, 1, 2)?;
    if parts.next().is_some() {
        return None;
    }
    let nano = match frac {
        Some(f) => number(f, 1, 9)? * 10u32.pow(9 - f.len() as u32),
        None => 0,
    };
    if sec == 60 {
        NaiveTime::from_hms_nano_opt(h, m, 59, 1_000_000_000 + nano)
    } else {
        NaiveTime::from_hms_nano_opt(h, m, sec, nano)
    }
}

fn resolve_zone(naive: NaiveDateTime, zone: &str) -> Option<DateTime<Utc>> {
    if ["UTC", "GMT", "Z"]
        .iter()
        .any(|z| zone.eq_ignore_ascii_case(z))
    {
        return Some(Utc.from_utc_datetime(&naive));
    }
    if let Some(rest) = zone.strip_prefix(['+', '-']) {
        let (hh, mm) = match rest.split_once(':') {
            Some(hm) => hm,
            None if rest.len() == 4 => rest.split_at(2),
            None => return None,
        };
        let secs = (number(hh, 2, 2)? * 3600 + number(mm, 2, 2)? * 60) as i32;
        let offset = FixedOffset::east_opt(if zone.starts_with('-') { -secs } else { secs })?;
        return offset
            .from_local_datetime(&naive)
            .single()
            .map(|t| t.with_timezone(&Utc));
    }
    let tz = chrono_tz::Tz::from_str(zone).ok()?;
    tz.from_local_datetime(&naive)
        .earliest()
        .map(|t| t.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        let utc = parse_time("Sep 19 2026 08:26:10 UTC").unwrap();
        assert_eq!(utc.to_rfc3339(), "2026-09-19T08:26:10+00:00");
        let frac = parse_time("Sep 19 2026 08:26:10.250 GMT").unwrap();
        assert_eq!(frac.timestamp_millis() - utc.timestamp_millis(), 250);
        let offset = parse_time("Sep 19 2026 15:26:10 +07:00").unwrap();
        assert_eq!(offset, utc);
        let iana = parse_time("Sep 19 2026 15:26:10 Asia/Jakarta").unwrap();
        assert_eq!(iana, utc);

        let local = parse_time("Sep 19 2026 08:26:10").unwrap();
        let expected = Local
            .with_ymd_and_hms(2026, 9, 19, 8, 26, 10)
            .earliest()
            .unwrap();
        assert_eq!(local, expected.with_timezone(&Utc));

        let no_year = parse_time("Sep 19 08:26:10.123 UTC").unwrap();
        assert_eq!(no_year.year(), Local::now().year());
        assert_eq!(no_year.timestamp_subsec_millis(), 123);
    }

    #[test]
    fn nxos_order() {
        let t = parse_time("2026 Jan 12 03:04:05 UTC").unwrap();
        assert_eq!(t.to_rfc3339(), "2026-01-12T03:04:05+00:00");
        let t = parse_time("2026 Jan 12 03:04:05.250 +07:00").unwrap();
        assert_eq!(t.to_rfc3339(), "2026-01-11T20:04:05.250+00:00");
    }

    #[test]
    fn hand_parser_edges() {
        let t = |s| parse_time(s).map(|t| t.to_rfc3339());
        assert_eq!(
            t("january 12 2026 03:04:05 UTC").as_deref(),
            Some("2026-01-12T03:04:05+00:00")
        );
        assert_eq!(
            t("JAN 2 2026 3:04:05.123456789 +0700").as_deref(),
            Some("2026-01-01T20:04:05.123456789+00:00")
        );
        assert_eq!(
            t("Dec 31 2026 23:59:60 UTC").as_deref(),
            Some("2026-12-31T23:59:60+00:00")
        );
        for bad in [
            "Feb 30 2026 03:04:05 UTC",
            "Jan 12 2026 25:04:05 UTC",
            "Jan 12 2026 03:04 UTC",
            "Jan 12 2026 03:04:05. UTC",
            "Jan 12 2026 03:04:05.1234567890 UTC",
            "Ja 12 2026 03:04:05 UTC",
            "Janu 12 2026 03:04:05 UTC",
            "Jan 123 2026 03:04:05 UTC",
            "Jan 12 2026 03:04:05 +07",
            "Jan 12 2026 03:04:05 UTC trailing",
        ] {
            assert_eq!(t(bad), None, "{bad}");
        }
    }

    #[test]
    fn padded_day_and_unknown_zone() {
        // IOS pads a one-digit day with a space.
        let t = parse_time("Mar  1 2026 00:01:02.345 UTC").unwrap();
        assert_eq!(t.to_rfc3339(), "2026-03-01T00:01:02.345+00:00");
        assert!(parse_time("Jan 12 2026 03:04:05 XYZ").is_none());
        assert!(parse_time("not a time").is_none());
    }
}

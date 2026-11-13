// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! DNS responses as [`Outcome`]s.

use hickory_resolver::net::{DnsError, NetError};
use hickory_resolver::proto::op::ResponseCode;
use hickory_resolver::proto::rr::rdata::SOA;
use hickory_resolver::proto::rr::{Name, RData, Record, RecordType};
use patrolli::cache::dns::{Answer, Outcome, Question};
use std::time::Duration;

/// What is put on the wire for a question.
pub(crate) fn query(question: &Question) -> (Name, RecordType) {
    match question {
        // `in-addr.arpa` / `ip6.arpa`, fully qualified
        Question::Ptr(ip) => (Name::from(*ip), RecordType::PTR),
    }
}

/// The sections of a response, read for `question`.
///
/// The first matching record wins, with the smallest TTL among them; a name
/// loses its trailing dot. No matching record is a negative answer, with the
/// SOA's TTL when the authority section carries one.
pub(crate) fn from_records(
    question: &Question,
    answers: &[Record],
    authorities: &[Record],
) -> Outcome {
    let mut answer = None;
    let mut ttl = u32::MAX;
    for record in answers {
        let found = match (question, &record.data) {
            (Question::Ptr(_), RData::PTR(ptr)) => {
                let name = ptr.0.to_utf8();
                Answer::Name(name.strip_suffix('.').unwrap_or(&name).into())
            }
            _ => continue,
        };
        ttl = ttl.min(record.ttl);
        answer.get_or_insert(found);
    }
    match answer {
        Some(answer) => Outcome::Found {
            answer,
            ttl: Duration::from_secs(ttl.into()),
        },
        None => Outcome::Negative {
            ttl: authorities.iter().find_map(|record| match &record.data {
                RData::SOA(soa) => Some(negative_ttl(record.ttl, soa)),
                _ => None,
            }),
        },
    }
}

/// A stub resolver's error: "no such name / record" is an answer, anything
/// else a failure.
pub(crate) fn from_net_error(error: &NetError) -> Option<Outcome> {
    let NetError::Dns(DnsError::NoRecordsFound(no_records)) = error else {
        return None;
    };
    let ttl = match (no_records.negative_ttl, &no_records.soa) {
        (Some(ttl), _) => Some(Duration::from_secs(ttl.into())),
        (None, Some(soa)) => Some(negative_ttl(soa.ttl, &soa.data)),
        (None, None) => None,
    };
    Some(Outcome::Negative { ttl })
}

/// Whether a stub resolver's error is the server answering "server failure":
/// it tried and could not resolve the name.
pub(crate) fn is_server_failure(error: &NetError) -> bool {
    matches!(
        error,
        NetError::Dns(DnsError::ResponseCode(ResponseCode::ServFail))
    )
}

/// Negative-cache TTL per RFC 2308: the lesser of the SOA record's TTL and
/// its MINIMUM.
pub(crate) fn negative_ttl(soa_record_ttl: u32, soa: &SOA) -> Duration {
    Duration::from_secs(soa_record_ttl.min(soa.minimum).into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hickory_resolver::proto::rr::rdata::PTR;

    fn question() -> Question {
        Question::Ptr("8.8.8.8".parse().unwrap())
    }

    fn ptr(name: &str, ttl: u32) -> Record {
        Record::from_rdata(
            Name::from_ascii("8.8.8.8.in-addr.arpa.").unwrap(),
            ttl,
            RData::PTR(PTR(Name::from_ascii(name).unwrap())),
        )
    }

    #[test]
    fn the_first_ptr_wins_with_the_smallest_ttl_and_no_trailing_dot() {
        let answers = [ptr("dns.google.", 600), ptr("other.example.", 120)];
        assert_eq!(
            from_records(&question(), &answers, &[]),
            Outcome::Found {
                answer: Answer::Name("dns.google".into()),
                ttl: Duration::from_secs(120)
            }
        );
    }

    #[test]
    fn no_ptr_is_negative_with_the_soa_ttl() {
        let soa = SOA::new(
            Name::from_ascii("ns.example.").unwrap(),
            Name::from_ascii("admin.example.").unwrap(),
            1,
            7200,
            900,
            1_209_600,
            180,
        );
        let authority = Record::from_rdata(
            Name::from_ascii("8.in-addr.arpa.").unwrap(),
            3600,
            RData::SOA(soa),
        );
        assert_eq!(
            from_records(&question(), &[], &[authority]),
            Outcome::Negative {
                ttl: Some(Duration::from_secs(180))
            }
        );
        assert_eq!(
            from_records(&question(), &[], &[]),
            Outcome::Negative { ttl: None }
        );
    }

    #[test]
    fn a_reverse_name_is_fully_qualified() {
        let (name, ty) = query(&question());
        assert_eq!(name.to_string(), "8.8.8.8.in-addr.arpa.");
        assert_eq!(ty, RecordType::PTR);
        let (v6, _) = query(&Question::Ptr("2001:db8::1".parse().unwrap()));
        assert_eq!(v6.num_labels(), 34);
    }
}

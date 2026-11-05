// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! What the sessions tell the pipeline: lifecycle events, and with
//! `events.routes` one record per route change. Each becomes a log record
//! whose source address is the router, under `bgp.*` attributes.
//!
//! The table never waits on these: they are written after the routes, and
//! whether a full channel drops them or holds the session up is the
//! listener's `events.on_full`.

use super::routers::Protocol;
use super::wire::bgp::{CODE_NAMES, code_index};
use crate::cache::lookup_table::bgp::Family;
use crate::cache::lookup_table::cell::CellOwned;
use crate::config::listener::OnFull;
use crate::config::lookup_table::bgp as columns;
use crate::payload::{LogRow, LogsBuilder, Value, ZString};
use chrono::{DateTime, Utc};
use flume::{Sender, TrySendError};
use ipnet::IpNet;
use opentelemetry_proto::tonic::logs::v1::SeverityNumber;
use smol_str::SmolStr;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio_util::sync::CancellationToken;

/// One record for the pipeline.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub time: DateTime<Utc>,
    pub router: IpAddr,
    /// the session's protocol; none for routes whose session is unknown
    pub protocol: Option<Protocol>,
    pub kind: EventKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EventKind {
    SessionUp {
        /// the router's previous session was still live and was ended
        superseded: bool,
    },
    SessionDown {
        /// [`super::SessionEnd::as_str`]
        reason: &'static str,
        error: Option<&'static str>,
        /// the BGP NOTIFICATION that ended the session, sent or received, as
        /// (code, subcode)
        notification: Option<(u8, u8)>,
        /// the router's shutdown communication (RFC 9003)
        message: Option<SmolStr>,
    },
    PeerUp {
        vrf: SmolStr,
        asn: u32,
        bgp_id: Ipv4Addr,
    },
    PeerDown {
        vrf: SmolStr,
        asn: u32,
        bgp_id: Ipv4Addr,
        rows: u64,
    },
    EndOfRib {
        vrf: SmolStr,
        family: Family,
        rows: u64,
    },
    /// the router stayed away past `keep`, or did not re-send these routes
    KeepExpired { rows: u64 },
    /// `cells` are the route's row cells (see
    /// [`crate::cache::lookup_table::bgp::RouteRow::cells`])
    Announce {
        prefix: IpNet,
        cells: Box<[CellOwned]>,
    },
    Withdraw {
        vrf: SmolStr,
        prefix: IpNet,
        path_id: Option<u32>,
    },
}

impl EventKind {
    pub fn name(&self) -> &'static str {
        match self {
            Self::SessionUp { .. } => "session_up",
            Self::SessionDown { .. } => "session_down",
            Self::PeerUp { .. } => "peer_up",
            Self::PeerDown { .. } => "peer_down",
            Self::EndOfRib { .. } => "end_of_rib",
            Self::KeepExpired { .. } => "keep_expired",
            Self::Announce { .. } => "announce",
            Self::Withdraw { .. } => "withdraw",
        }
    }

    /// Whether this is a route event, sent only with `events.routes`.
    pub fn is_route(&self) -> bool {
        matches!(self, Self::Announce { .. } | Self::Withdraw { .. })
    }
}

fn family_name(family: Family) -> &'static str {
    match family {
        Family::V4 => "ipv4",
        Family::V6 => "ipv6",
    }
}

/// The names of a route row's cells, in cell order: every column but the
/// prefix.
fn cell_names() -> impl Iterator<Item = &'static str> {
    columns::COLUMNS
        .iter()
        .map(|(name, _)| *name)
        .filter(|name| *name != columns::PREFIX)
}

impl Event {
    pub fn new(router: IpAddr, kind: EventKind) -> Self {
        Self {
            time: Utc::now(),
            router,
            protocol: None,
            kind,
        }
    }

    pub fn with_protocol(mut self, protocol: Protocol) -> Self {
        self.protocol = Some(protocol);
        self
    }

    fn severity(&self) -> SeverityNumber {
        match &self.kind {
            EventKind::SessionDown {
                reason: "frame" | "reset" | "no_initiation" | "open_rejected" | "hold_timer_expired",
                ..
            } => SeverityNumber::Warn,
            // a router ends a session it is unhappy with by anything but Cease
            EventKind::SessionDown {
                reason: "notification",
                notification: Some((code, _)),
                ..
            } if *code != super::wire::bgp::code::CEASE => SeverityNumber::Warn,
            _ => SeverityNumber::Info,
        }
    }

    /// Appends the event to `batch` as one log record.
    pub fn push(&self, batch: &mut LogsBuilder) {
        let s = |v: &'static str| Value::String(ZString::from_static(v));
        let text = |v: String| Value::String(ZString::from_owned(v));
        let int = |v: u64| Value::Int64(v.try_into().unwrap_or(i64::MAX));
        let mut fields: Vec<(ZString<'_>, Value<'_>)> = Vec::with_capacity(12);
        macro_rules! field {
            ($name:expr, $value:expr) => {
                fields.push((ZString::from_static($name), $value))
            };
        }
        field!("event", s(self.kind.name()));
        if let Some(protocol) = self.protocol {
            field!("protocol", s(protocol.as_str()));
        }
        match &self.kind {
            EventKind::SessionUp { superseded } => field!("superseded", Value::Bool(*superseded)),
            EventKind::SessionDown {
                reason,
                error,
                notification,
                message,
            } => {
                field!("reason", s(reason));
                if let Some(error) = error {
                    field!("error", s(error));
                }
                if let Some((code, subcode)) = notification {
                    field!("notification", text(format!("{code}/{subcode}")));
                    field!("notification_name", s(CODE_NAMES[code_index(*code)]));
                }
                if let Some(message) = message {
                    field!("message", Value::String(ZString::from_ref(message)));
                }
            }
            EventKind::PeerUp { vrf, asn, bgp_id } => {
                field!(columns::VRF, Value::String(ZString::from_ref(vrf)));
                field!("peer_as", int((*asn).into()));
                field!("bgp_id", text(bgp_id.to_string()));
            }
            EventKind::PeerDown {
                vrf,
                asn,
                bgp_id,
                rows,
            } => {
                field!(columns::VRF, Value::String(ZString::from_ref(vrf)));
                field!("peer_as", int((*asn).into()));
                field!("bgp_id", text(bgp_id.to_string()));
                field!("rows_removed", int(*rows));
            }
            EventKind::EndOfRib { vrf, family, rows } => {
                field!(columns::VRF, Value::String(ZString::from_ref(vrf)));
                field!("family", s(family_name(*family)));
                field!("rows_swept", int(*rows));
            }
            EventKind::KeepExpired { rows } => field!("rows_swept", int(*rows)),
            EventKind::Announce { prefix, cells } => {
                field!(columns::PREFIX, text(prefix.to_string()));
                for (name, cell) in cell_names().zip(cells.iter()) {
                    if name == columns::ROUTER || matches!(cell, CellOwned::Null) {
                        continue;
                    }
                    field!(name, cell.as_ref().to_value());
                }
            }
            EventKind::Withdraw {
                vrf,
                prefix,
                path_id,
            } => {
                field!(columns::VRF, Value::String(ZString::from_ref(vrf)));
                field!(columns::PREFIX, text(prefix.to_string()));
                if let Some(id) = path_id {
                    field!(columns::PATH_ID, int((*id).into()));
                }
            }
        }
        let attrs = [(ZString::from_static("bgp"), Value::KvList(fields))];
        batch.push_row(LogRow {
            observed_time: Some(self.time),
            severity: Some(self.severity()),
            body: None,
            source_ip: Some(self.router),
            source_port: None,
            attrs: &attrs,
        });
    }
}

/// Where the sessions send their events.
#[derive(Debug)]
pub struct EventSink {
    tx: Sender<Event>,
    routes: bool,
    on_full: OnFull,
    /// events lost to a full or closed channel; the listener's, so it
    /// outlives a transport restart
    dropped: Arc<AtomicU64>,
}

impl EventSink {
    pub fn new(tx: Sender<Event>, routes: bool, on_full: OnFull, dropped: Arc<AtomicU64>) -> Self {
        Self {
            tx,
            routes,
            on_full,
            dropped,
        }
    }

    /// Events lost so far.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Whether route events are wanted.
    pub fn routes(&self) -> bool {
        self.routes
    }

    /// Sends without waiting, whatever `on_full` says.
    pub fn try_send(&self, event: Event) {
        if let Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) = self.tx.try_send(event)
        {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Sends `events` in order. With [`OnFull::Wait`] this waits for room
    /// until `cancel`, and sends what is left without waiting after it.
    pub async fn send_all(&self, events: &mut Vec<Event>, cancel: &CancellationToken) {
        for event in events.drain(..) {
            if self.on_full == OnFull::Drop || cancel.is_cancelled() {
                self.try_send(event);
                continue;
            }
            let sent = tokio::select! {
                biased;
                _ = cancel.cancelled() => false,
                sent = self.tx.send_async(event) => sent.is_ok(),
            };
            if !sent {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::lookup_table::bgp::{PathSegment, RouteRow, RouteText};
    use crate::cache::lookup_table::cell::CellRef;
    use serde_json::{Value as Json, json};

    /// Each event's record attributes and severity.
    fn records(events: &[Event]) -> Vec<(Json, i64)> {
        let mut batch = LogsBuilder::new();
        for e in events {
            e.push(&mut batch);
        }
        batch
            .finish()
            .serialize_as(crate::config::Format::JsonNestedDenormalized)
            .unwrap()
            .iter()
            .flat_map(|b| {
                std::str::from_utf8(b)
                    .unwrap()
                    .lines()
                    .map(|l| {
                        let v: Json = serde_json::from_str(l).unwrap();
                        let record = &v["record"];
                        (
                            record["attributes"].clone(),
                            record["severityNumber"].as_i64().unwrap(),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    #[test]
    fn route_events_carry_the_row_columns_without_nulls() {
        let router: IpAddr = "192.0.2.1".parse().unwrap();
        let path = [PathSegment::Sequence(&[64501, 64502])];
        let route = RouteRow {
            router,
            vrf: "global",
            prefix: "10.0.0.0/8".parse().unwrap(),
            next_hop: Some("192.0.2.9".parse().unwrap()),
            as_path: &path,
            communities: &[(64501 << 16) | 7],
            large_communities: &[],
            local_pref: Some(100),
            med: None,
            path_id: None,
        };
        let cells = route
            .cells(&mut RouteText::default())
            .map(CellRef::to_owned_cell);
        let got = records(&[
            Event::new(
                router,
                EventKind::Announce {
                    prefix: route.prefix,
                    cells: Box::new(cells),
                },
            ),
            Event::new(
                router,
                EventKind::Withdraw {
                    vrf: "65000:1".into(),
                    prefix: "2001:db8::/32".parse().unwrap(),
                    path_id: Some(2),
                },
            ),
        ]);
        assert_eq!(
            got[0].0,
            json!({
                "source": {"address": "192.0.2.1"},
                "bgp": {
                    "event": "announce",
                    "prefix": "10.0.0.0/8",
                    "vrf": "global",
                    "next_hop": "192.0.2.9",
                    "origin_as": 64502,
                    "peer_as": 64501,
                    "as_path": "64501 64502",
                    "communities": "64501:7",
                    "local_pref": 100,
                },
            })
        );
        assert_eq!(
            got[1].0["bgp"],
            json!({"event": "withdraw", "vrf": "65000:1", "prefix": "2001:db8::/32", "path_id": 2})
        );
    }

    #[test]
    fn a_session_lost_to_bad_input_is_a_warning() {
        let router: IpAddr = "192.0.2.1".parse().unwrap();
        let down = |reason, error| {
            Event::new(
                router,
                EventKind::SessionDown {
                    reason,
                    error,
                    notification: None,
                    message: None,
                },
            )
        };
        let got = records(&[
            Event::new(router, EventKind::SessionUp { superseded: true }),
            down("eof", None),
            down("reset", Some("attribute truncated")),
            Event::new(
                router,
                EventKind::EndOfRib {
                    vrf: "global".into(),
                    family: Family::V6,
                    rows: 3,
                },
            ),
        ]);
        let warn = SeverityNumber::Warn as i64;
        let info = SeverityNumber::Info as i64;
        assert_eq!(
            got[0],
            (
                json!({"source": {"address": "192.0.2.1"}, "bgp": {"event": "session_up", "superseded": true}}),
                info
            )
        );
        assert_eq!(got[1].1, info);
        assert_eq!(
            got[2],
            (
                json!({"source": {"address": "192.0.2.1"}, "bgp": {"event": "session_down", "reason": "reset", "error": "attribute truncated"}}),
                warn
            )
        );
        assert_eq!(
            got[3].0["bgp"],
            json!({"event": "end_of_rib", "vrf": "global", "family": "ipv6", "rows_swept": 3})
        );
    }

    #[test]
    fn a_bgp_session_down_names_its_notification() {
        let router: IpAddr = "192.0.2.1".parse().unwrap();
        let down = |notification, message: Option<&str>| {
            Event::new(
                router,
                EventKind::SessionDown {
                    reason: "notification",
                    error: None,
                    notification: Some(notification),
                    message: message.map(SmolStr::from),
                },
            )
            .with_protocol(Protocol::Bgp)
        };
        let got = records(&[down((6, 2), Some("maintenance")), down((3, 1), None)]);
        assert_eq!(
            got[0],
            (
                json!({"source": {"address": "192.0.2.1"}, "bgp": {
                    "event": "session_down",
                    "protocol": "bgp",
                    "reason": "notification",
                    "notification": "6/2",
                    "notification_name": "cease",
                    "message": "maintenance",
                }}),
                SeverityNumber::Info as i64
            )
        );
        assert_eq!(got[1].1, SeverityNumber::Warn as i64);
    }
}

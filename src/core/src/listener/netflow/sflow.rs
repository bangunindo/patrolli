// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! sFlow v5 ingest, sharing the NetFlow listener's UDP port.
//!
//! sFlow and NetFlow commonly arrive on the same UDP port. They are told apart
//! by their version field width: NetFlow/IPFIX put a 2-byte version first (so
//! byte 1 is non-zero), while sFlow v5 puts a 4-byte version (== 5), so its
//! byte 1 is always zero. See [`is_sflow`].

use crate::payload::{self, LogRow, Value, ZString};
use sflow_parser::SFlowDatagram;
use sflow_parser::models::{
    Address, CounterData, FlowData, FlowRecord, GenericInterfaceCounters, HeaderProtocol,
    SampleData, SampledHeader,
};
use smol_str::{ToSmolStr, format_smolstr};
use std::borrow::Cow;
use std::net::{IpAddr, SocketAddr};

/// Returns true if the datagram is an sFlow v5 packet.
///
/// sFlow v5 encodes its version as a 4-byte big-endian integer equal to 5,
/// so bytes 0..2 are always zero. NetFlow/IPFIX encode a 2-byte version
/// (5/7/9/10) in the first two bytes, so their byte 1 is never zero. Reading
/// the first four bytes as a u32 therefore disambiguates the two without
/// colliding with NetFlow v5 (whose first four bytes are `00 05 <count..>`).
pub fn is_sflow(buf: &[u8]) -> bool {
    buf.len() >= 4 && u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) == 5
}

/// Convert a parsed sFlow datagram into payload log rows, pushing one row per
/// flow sample and one per counter record onto `batcher`. The datagram is one
/// item, succeeded however many of its samples are dropped, as a NetFlow v9
/// or IPFIX packet is.
pub fn datagram_to_logs(
    dg: SFlowDatagram,
    addr: SocketAddr,
    batcher: &mut payload::LogsBuilder,
    tally: &mut super::Tally,
) {
    tally.items_succeeded += 1;
    let sub_agent_id = dg.sub_agent_id;
    let uptime = dg.uptime as f64 / 1000.0;
    for sample in dg.samples {
        match sample.sample_data {
            SampleData::FlowSample(s) => emit_flow_sample(
                batcher,
                addr,
                tally,
                sub_agent_id,
                uptime,
                s.sequence_number,
                s.sampling_rate,
                s.sample_pool,
                s.drops,
                s.input.value(),
                s.output.value(),
                s.flow_records,
            ),
            SampleData::FlowSampleExpanded(s) => emit_flow_sample(
                batcher,
                addr,
                tally,
                sub_agent_id,
                uptime,
                s.sequence_number,
                s.sampling_rate,
                s.sample_pool,
                s.drops,
                s.input.value,
                s.output.value,
                s.flow_records,
            ),
            SampleData::CountersSample(s) => {
                for record in s.counters {
                    emit_counter_record(
                        batcher,
                        addr,
                        tally,
                        sub_agent_id,
                        uptime,
                        record.counter_data,
                    );
                }
            }
            SampleData::CountersSampleExpanded(s) => {
                for record in s.counters {
                    emit_counter_record(
                        batcher,
                        addr,
                        tally,
                        sub_agent_id,
                        uptime,
                        record.counter_data,
                    );
                }
            }
            _ => {
                tally.records_dropped += 1;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_flow_sample(
    batcher: &mut payload::LogsBuilder,
    addr: SocketAddr,
    tally: &mut super::Tally,
    sub_agent_id: u32,
    uptime: f64,
    sequence_number: u32,
    sampling_rate: u32,
    sample_pool: u32,
    drops: u32,
    input: u32,
    output: u32,
    flow_records: Vec<FlowRecord>,
) {
    tally.records += 1;
    let mut attrs: Vec<(ZString, Value)> = Vec::with_capacity(16);
    attrs.push((
        ZString::from_static("nf.data.sample_pool"),
        Value::Int64(sample_pool as i64),
    ));
    attrs.push((
        ZString::from_static("nf.data.drops"),
        Value::Int64(drops as i64),
    ));
    attrs.push((
        ZString::from_static("nf.data.input"),
        Value::Int64(input as i64),
    ));
    attrs.push((
        ZString::from_static("nf.data.output"),
        Value::Int64(output as i64),
    ));
    // Borrow the raw sampled-header bytes out of `flow_records`, so the records
    // must stay alive until after `push_row` copies them into the batch.
    for flow_record in &flow_records {
        add_flow_record(&mut attrs, &flow_record.flow_data);
    }
    super::push_header(
        &mut attrs,
        "sflow",
        Some(sequence_number),
        Some(sampling_rate),
        Some(sub_agent_id),
        Some("flow_sample"),
        Some(uptime),
    );
    batcher.push_row(flow_row(addr, &attrs));
}

// TODO: FlowData is not exhaustive yet
fn add_flow_record<'a>(attrs: &mut Vec<(ZString<'a>, Value<'a>)>, data: &'a FlowData) {
    match data {
        FlowData::SampledHeader(header) => add_sampled_header(attrs, header),
        FlowData::ExtendedSwitch(s) => {
            attrs.push((
                ZString::from_static("nf.data.extended_switch.src_vlan"),
                Value::Int64(s.src_vlan as i64),
            ));
            attrs.push((
                ZString::from_static("nf.data.extended_switch.src_priority"),
                Value::Int64(s.src_priority as i64),
            ));
            attrs.push((
                ZString::from_static("nf.data.extended_switch.dst_vlan"),
                Value::Int64(s.dst_vlan as i64),
            ));
            attrs.push((
                ZString::from_static("nf.data.extended_switch.dst_priority"),
                Value::Int64(s.dst_priority as i64),
            ));
        }
        FlowData::ExtendedRouter(r) => {
            if let Some(ip) = address_to_ip(&r.next_hop) {
                attrs.push((
                    ZString::from_static("nf.data.extended_router.next_hop"),
                    Value::String(ZString::from_smol(ip.to_smolstr())),
                ));
            }
            attrs.push((
                ZString::from_static("nf.data.extended_router.src_mask_len"),
                Value::Int64(r.src_mask_len as i64),
            ));
            attrs.push((
                ZString::from_static("nf.data.extended_router.dst_mask_len"),
                Value::Int64(r.dst_mask_len as i64),
            ));
        }
        FlowData::ExtendedGateway(g) => {
            if let Some(ip) = address_to_ip(&g.next_hop) {
                attrs.push((
                    ZString::from_static("nf.data.extended_gateway.next_hop"),
                    Value::String(ZString::from_smol(ip.to_smolstr())),
                ));
            }
            attrs.push((
                ZString::from_static("nf.data.extended_gateway.as"),
                Value::Int64(g.as_number as i64),
            ));
            attrs.push((
                ZString::from_static("nf.data.extended_gateway.src_as"),
                Value::Int64(g.src_as as i64),
            ));
            attrs.push((
                ZString::from_static("nf.data.extended_gateway.src_peer_as"),
                Value::Int64(g.src_peer_as as i64),
            ));
            attrs.push((
                ZString::from_static("nf.data.extended_gateway.local_pref"),
                Value::Int64(g.local_pref as i64),
            ));
            let communities: Vec<Value> = g
                .communities
                .iter()
                .map(|c| Value::Int64(*c as i64))
                .collect();
            attrs.push((
                ZString::from_static("nf.data.extended_gateway.communities"),
                Value::Array(communities),
            ));
            let as_path: Vec<Value> = g
                .dst_as_path
                .iter()
                .map(|seg| {
                    Value::KvList(vec![
                        (
                            ZString::from_ref("path_type"),
                            Value::String(ZString::from_smol(format_smolstr!(
                                "{:?}",
                                seg.path_type
                            ))),
                        ),
                        (
                            ZString::from_ref("path_length"),
                            Value::Int64(seg.path_length as i64),
                        ),
                        (
                            ZString::from_ref("path"),
                            Value::Array(
                                seg.path.iter().map(|a| Value::Int64(*a as i64)).collect(),
                            ),
                        ),
                    ])
                })
                .collect();
            attrs.push((
                ZString::from_static("nf.data.extended_gateway.dst_as_path"),
                Value::Array(as_path),
            ));
        }
        _ => {}
    }
}

fn add_sampled_header<'a>(attrs: &mut Vec<(ZString<'a>, Value<'a>)>, header: &'a SampledHeader) {
    attrs.push((
        ZString::from_static("nf.data.protocol"),
        Value::String(ZString::from_smol(format_smolstr!("{:?}", header.protocol))),
    ));
    attrs.push((
        ZString::from_static("nf.data.frame_length"),
        Value::Int64(header.frame_length as i64),
    ));
    attrs.push((
        ZString::from_static("nf.data.stripped"),
        Value::Int64(header.stripped as i64),
    ));
    decode_inner_packet(attrs, header.protocol, &header.header);
    attrs.push((
        ZString::from_static("nf.data.header"),
        Value::Bytes(Cow::Borrowed(&header.header)),
    ));
}

/// Decode the sampled raw packet header into a 5-tuple when possible. Only the
/// link types we can interpret (Ethernet, raw IPv4/IPv6) are decoded; anything
/// else leaves only the raw `header` bytes on the record.
fn decode_inner_packet<'a>(
    attrs: &mut Vec<(ZString<'a>, Value<'a>)>,
    protocol: HeaderProtocol,
    header: &[u8],
) {
    use etherparse::{NetSlice, SlicedPacket, TransportSlice};

    let sliced = match protocol {
        HeaderProtocol::EthernetIso88023 => SlicedPacket::from_ethernet(header),
        HeaderProtocol::Ipv4 | HeaderProtocol::Ipv6 => SlicedPacket::from_ip(header),
        _ => return,
    };
    let Ok(sliced) = sliced else { return };

    match sliced.net {
        Some(NetSlice::Ipv4(ip)) => {
            let h = ip.header();
            attrs.push((
                ZString::from_static("nf.data.src_ip"),
                Value::String(ZString::from_smol(IpAddr::V4(h.source_addr()).to_smolstr())),
            ));
            attrs.push((
                ZString::from_static("nf.data.dst_ip"),
                Value::String(ZString::from_smol(
                    IpAddr::V4(h.destination_addr()).to_smolstr(),
                )),
            ));
            attrs.push((
                ZString::from_static("nf.data.ip_protocol"),
                Value::Int64(h.protocol().0 as i64),
            ));
        }
        Some(NetSlice::Ipv6(ip)) => {
            let h = ip.header();
            attrs.push((
                ZString::from_static("nf.data.src_ip"),
                Value::String(ZString::from_smol(IpAddr::V6(h.source_addr()).to_smolstr())),
            ));
            attrs.push((
                ZString::from_static("nf.data.dst_ip"),
                Value::String(ZString::from_smol(
                    IpAddr::V6(h.destination_addr()).to_smolstr(),
                )),
            ));
            attrs.push((
                ZString::from_static("nf.data.ip_protocol"),
                Value::Int64(h.next_header().0 as i64),
            ));
        }
        _ => {}
    }

    match sliced.transport {
        Some(TransportSlice::Tcp(t)) => {
            attrs.push((
                ZString::from_static("nf.data.src_port"),
                Value::Int64(t.source_port() as i64),
            ));
            attrs.push((
                ZString::from_static("nf.data.dst_port"),
                Value::Int64(t.destination_port() as i64),
            ));
        }
        Some(TransportSlice::Udp(u)) => {
            attrs.push((
                ZString::from_static("nf.data.src_port"),
                Value::Int64(u.source_port() as i64),
            ));
            attrs.push((
                ZString::from_static("nf.data.dst_port"),
                Value::Int64(u.destination_port() as i64),
            ));
        }
        _ => {}
    }
}

fn emit_counter_record(
    batcher: &mut payload::LogsBuilder,
    addr: SocketAddr,
    tally: &mut super::Tally,
    sub_agent_id: u32,
    uptime: f64,
    data: CounterData,
) {
    match data {
        CounterData::GenericInterface(c) => {
            tally.records += 1;
            let mut attrs: Vec<(ZString, Value)> = Vec::with_capacity(16);
            push_generic_interface(&mut attrs, &c);
            super::push_header(
                &mut attrs,
                "sflow",
                None,
                None,
                Some(sub_agent_id),
                Some("counters:interface"),
                Some(uptime),
            );
            batcher.push_row(flow_row(addr, &attrs));
        }
        // Other counter records (ethernet dot3 stats, host/VM/app counters, ...)
        // are carried verbatim under a `counters:<kind>` flow set id.
        other => {
            tally.records += 1;
            let (flow_set_id, mut attrs) = counter_fallback_attrs(other);
            super::push_header(
                &mut attrs,
                "sflow",
                None,
                None,
                Some(sub_agent_id),
                Some(flow_set_id),
                Some(uptime),
            );
            batcher.push_row(flow_row(addr, &attrs));
        }
    }
}

/// Source endpoint shared by every sFlow row. sFlow carries no per-sample
/// wall-clock time, so `observed_time` is left for `push_row` to default.
fn flow_row<'a, 'b: 'a>(addr: SocketAddr, attrs: &'b Vec<(ZString<'a>, Value<'a>)>) -> LogRow<'a> {
    LogRow {
        observed_time: None,
        severity: None,
        body: None,
        source_ip: Some(addr.ip()),
        source_port: Some(addr.port()),
        attrs,
    }
}

/// Normalized generic interface counters, matching the field names the old
/// `Interface` metric exposed (rx/tx byte & packet totals, up/down statuses).
fn push_generic_interface<'a>(
    attrs: &mut Vec<(ZString<'a>, Value<'a>)>,
    c: &GenericInterfaceCounters,
) {
    // if_status: bit 0 = admin up, bit 1 = oper up.
    let status = |bit: u32| -> &'static str { if c.if_status & bit != 0 { "up" } else { "down" } };
    attrs.push((
        ZString::from_static("nf.data.index"),
        Value::Int64(c.if_index as i64),
    ));
    attrs.push((
        ZString::from_static("nf.data.type"),
        Value::Int64(c.if_type as i64),
    ));
    attrs.push((
        ZString::from_static("nf.data.link_speed"),
        Value::Int64(c.if_speed as i64),
    ));
    attrs.push((
        ZString::from_static("nf.data.admin_status"),
        Value::String(ZString::from_ref(status(0x1))),
    ));
    attrs.push((
        ZString::from_static("nf.data.operational_status"),
        Value::String(ZString::from_ref(status(0x2))),
    ));
    attrs.push((
        ZString::from_static("nf.data.rx_byte"),
        Value::Int64(c.if_in_octets as i64),
    ));
    attrs.push((
        ZString::from_static("nf.data.rx_pkts"),
        Value::Int64(
            (c.if_in_ucast_pkts as u64
                + c.if_in_multicast_pkts as u64
                + c.if_in_broadcast_pkts as u64) as i64,
        ),
    ));
    attrs.push((
        ZString::from_static("nf.data.rx_pkts_discard"),
        Value::Int64(c.if_in_discards as i64),
    ));
    attrs.push((
        ZString::from_static("nf.data.rx_pkts_error"),
        Value::Int64(c.if_in_errors as i64),
    ));
    attrs.push((
        ZString::from_static("nf.data.tx_byte"),
        Value::Int64(c.if_out_octets as i64),
    ));
    attrs.push((
        ZString::from_static("nf.data.tx_pkts"),
        Value::Int64(
            (c.if_out_ucast_pkts as u64
                + c.if_out_multicast_pkts as u64
                + c.if_out_broadcast_pkts as u64) as i64,
        ),
    ));
    attrs.push((
        ZString::from_static("nf.data.tx_pkts_discard"),
        Value::Int64(c.if_out_discards as i64),
    ));
    attrs.push((
        ZString::from_static("nf.data.tx_pkts_error"),
        Value::Int64(c.if_out_errors as i64),
    ));
}

/// Minimal verbatim representation for counter records that have no dedicated
/// shape. Returns the `counters:<kind>` flow set id and a list of the most
/// useful fields.
// TODO: CounterData is not exhaustive yet
fn counter_fallback_attrs<'a>(data: CounterData) -> (&'static str, Vec<(ZString<'a>, Value<'a>)>) {
    match data {
        CounterData::EthernetInterface(c) => {
            let attrs: Vec<(ZString, Value)> = vec![
                (
                    ZString::from_static("nf.data.alignment_errors"),
                    Value::Int64(c.dot3_stats_alignment_errors as i64),
                ),
                (
                    ZString::from_static("nf.data.fcs_errors"),
                    Value::Int64(c.dot3_stats_fcs_errors as i64),
                ),
                (
                    ZString::from_static("nf.data.single_collision_frames"),
                    Value::Int64(c.dot3_stats_single_collision_frames as i64),
                ),
                (
                    ZString::from_static("nf.data.multiple_collision_frames"),
                    Value::Int64(c.dot3_stats_multiple_collision_frames as i64),
                ),
                (
                    ZString::from_static("nf.data.sqe_test_errors"),
                    Value::Int64(c.dot3_stats_sqe_test_errors as i64),
                ),
                (
                    ZString::from_static("nf.data.deferred_transmissions"),
                    Value::Int64(c.dot3_stats_deferred_transmissions as i64),
                ),
                (
                    ZString::from_static("nf.data.late_collisions"),
                    Value::Int64(c.dot3_stats_late_collisions as i64),
                ),
                (
                    ZString::from_static("nf.data.excessive_collisions"),
                    Value::Int64(c.dot3_stats_excessive_collisions as i64),
                ),
                (
                    ZString::from_static("nf.data.internal_mac_transmit_errors"),
                    Value::Int64(c.dot3_stats_internal_mac_transmit_errors as i64),
                ),
                (
                    ZString::from_static("nf.data.carrier_sense_errors"),
                    Value::Int64(c.dot3_stats_carrier_sense_errors as i64),
                ),
                (
                    ZString::from_static("nf.data.frame_too_longs"),
                    Value::Int64(c.dot3_stats_frame_too_longs as i64),
                ),
                (
                    ZString::from_static("nf.data.internal_mac_receive_errors"),
                    Value::Int64(c.dot3_stats_internal_mac_receive_errors as i64),
                ),
                (
                    ZString::from_static("nf.data.symbol_errors"),
                    Value::Int64(c.dot3_stats_symbol_errors as i64),
                ),
            ];
            ("counters:ethernet", attrs)
        }
        _ => ("counters:other", Vec::new()),
    }
}

fn address_to_ip(addr: &Address) -> Option<IpAddr> {
    match addr {
        Address::IPv4(ip) => Some(IpAddr::V4(*ip)),
        Address::IPv6(ip) => Some(IpAddr::V6(*ip)),
        Address::Unknown => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;
    use etherparse::PacketBuilder;
    use serde_json::Value as JsonValue;
    use smol_str::SmolStr;

    fn u32b(v: u32) -> Vec<u8> {
        v.to_be_bytes().to_vec()
    }
    fn u64b(v: u64) -> Vec<u8> {
        v.to_be_bytes().to_vec()
    }
    // XDR opaque<>: u32 length prefix + data padded to a 4-byte boundary.
    fn opaque(data: &[u8]) -> Vec<u8> {
        let mut out = u32b(data.len() as u32);
        out.extend_from_slice(data);
        let pad = (4 - data.len() % 4) % 4;
        out.extend(std::iter::repeat_n(0u8, pad));
        out
    }
    fn concat(parts: &[Vec<u8>]) -> Vec<u8> {
        parts.concat()
    }

    fn ethernet_tcp_frame() -> Vec<u8> {
        let mut frame = Vec::new();
        PacketBuilder::ethernet2([1, 2, 3, 4, 5, 6], [6, 5, 4, 3, 2, 1])
            .ipv4([192, 168, 0, 1], [192, 168, 0, 2], 64)
            .tcp(1234, 80, 0, 0)
            .write(&mut frame, &[0u8; 8])
            .unwrap();
        frame
    }

    /// Build a minimal sFlow v5 datagram with one flow sample (sampled raw
    /// Ethernet/IPv4/TCP header) and one counter sample (generic interface).
    fn sample_datagram() -> Vec<u8> {
        let frame = ethernet_tcp_frame();
        // sampled_header: protocol=1 (ethernet), frame_length, stripped=0, header opaque
        let sampled_header = concat(&[u32b(1), u32b(frame.len() as u32), u32b(0), opaque(&frame)]);
        // flow_record: format=1 (raw header) + opaque(body)
        let flow_record = concat(&[u32b(1), opaque(&sampled_header)]);
        // flow_sample: seq, source_id, sampling_rate, sample_pool, drops, input, output, num_records, records
        let flow_sample = concat(&[
            u32b(7),    // sequence_number
            u32b(0),    // source_id
            u32b(1000), // sampling_rate
            u32b(50),   // sample_pool
            u32b(0),    // drops
            u32b(11),   // input ifindex
            u32b(22),   // output ifindex
            u32b(1),    // num flow records
            flow_record,
        ]);
        // sample_record: sample_type=1 (flow sample) + opaque(body)
        let flow_sample_record = concat(&[u32b(1), opaque(&flow_sample)]);

        // generic_interface_counters body
        let generic = concat(&[
            u32b(11),            // if_index
            u32b(6),             // if_type
            u64b(1_000_000_000), // if_speed
            u32b(1),             // if_direction
            u32b(3),             // if_status (admin up | oper up)
            u64b(1000),          // if_in_octets
            u32b(10),            // if_in_ucast_pkts
            u32b(1),             // if_in_multicast_pkts
            u32b(2),             // if_in_broadcast_pkts
            u32b(3),             // if_in_discards
            u32b(4),             // if_in_errors
            u32b(0),             // if_in_unknown_protos
            u64b(2000),          // if_out_octets
            u32b(20),            // if_out_ucast_pkts
            u32b(0),             // if_out_multicast_pkts
            u32b(0),             // if_out_broadcast_pkts
            u32b(5),             // if_out_discards
            u32b(6),             // if_out_errors
            u32b(0),             // if_promiscuous_mode
        ]);
        // counter_record: format=1 (generic interface) + opaque(body)
        let counter_record = concat(&[u32b(1), opaque(&generic)]);
        // counters_sample: seq, source_id, num_records, records
        let counters_sample = concat(&[u32b(8), u32b(0), u32b(1), counter_record]);
        // sample_record: sample_type=2 (counters sample) + opaque(body)
        let counters_sample_record = concat(&[u32b(2), opaque(&counters_sample)]);

        // datagram: version=5, agent address (type 1 ipv4), sub_agent_id, seq, uptime, num_samples, samples
        concat(&[
            u32b(5),
            u32b(1),
            vec![10, 0, 0, 1], // agent ipv4 10.0.0.1
            u32b(42),          // sub_agent_id
            u32b(100),         // sequence_number
            u32b(123456),      // uptime ms
            u32b(2),           // num samples
            flow_sample_record,
            counters_sample_record,
        ])
    }

    fn addr() -> SocketAddr {
        SocketAddr::new(IpAddr::V4([203, 0, 113, 7].into()), 6343)
    }

    #[test]
    fn detects_sflow_not_netflow() {
        assert!(is_sflow(&sample_datagram()));
        // NetFlow v5 packet: version 5 as a 2-byte field, then record count.
        assert!(!is_sflow(&[0x00, 0x05, 0x00, 0x01]));
        // NetFlow v9 / IPFIX.
        assert!(!is_sflow(&[0x00, 0x09, 0x00, 0x01]));
        assert!(!is_sflow(&[0x00, 0x0a, 0x00, 0x01]));
        assert!(!is_sflow(&[0x00, 0x00, 0x00]));
    }

    #[test]
    fn converts_flow_and_counter_samples() {
        let bytes = sample_datagram();
        let dg = sflow_parser::parse_datagram(&bytes).expect("datagram parses");
        let mut tally = super::super::Tally::default();
        let mut builder = payload::LogsBuilder::new();
        builder.scope(payload::Scope {
            name: SmolStr::new_static("sflow-test"),
            listener_id: None,
            listener_type: Some(SmolStr::new_static("netflow")),
            listener_name: Some(SmolStr::new_static("sflow-test")),
            monitor_type: None,
            node_id: config::NODE_ID.get().copied(),
        });
        datagram_to_logs(dg, addr(), &mut builder, &mut tally);
        let payload = builder.finish();

        // One flow sample + one counter record => two log rows.
        assert_eq!(payload.num_items(), 2);
        assert_eq!((tally.records, tally.records_dropped), (2, 0));
        // #46: the datagram itself used to land in no outcome bucket.
        assert_eq!(tally.items_succeeded, 1);

        let json: JsonValue = serde_json::from_slice(
            payload
                .serialize_as(config::Format::JsonOtlpBatch)
                .expect("serialize")[0]
                .as_ref(),
        )
        .expect("json");
        let text = serde_json::to_string(&json).unwrap();

        // Flow sample: sFlow header fields and the decoded inner 5-tuple.
        assert!(text.contains("\"sflow\""), "missing sflow type: {text}");
        assert!(
            text.contains("flow_sample"),
            "missing flow_sample id: {text}"
        );
        assert!(
            text.contains("192.168.0.1"),
            "missing decoded src ip: {text}"
        );
        assert!(
            text.contains("192.168.0.2"),
            "missing decoded dst ip: {text}"
        );
        // Counter sample: normalized generic-interface counters.
        assert!(
            text.contains("counters:interface"),
            "missing interface counters id: {text}"
        );
        assert!(
            text.contains("operational_status"),
            "missing interface status: {text}"
        );
    }
}

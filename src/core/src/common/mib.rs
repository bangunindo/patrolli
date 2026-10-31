// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::{payload, payload::ZString};
use anyhow::{Context, Result, anyhow};
use async_snmp::Value;
use mib_rs::{Loader, Mib, Oid, source};
use nom::{
    IResult, Parser as NomParser,
    bytes::complete::take,
    multi::{count, many0},
    number::complete::{be_i32, be_u8, be_u16, be_u32},
};
use smol_str::ToSmolStr;
use std::net::IpAddr;
use std::sync::OnceLock;
use tracing::{debug, warn};

static MODEL: OnceLock<Mib> = OnceLock::new();

#[tracing::instrument(skip_all)]
pub async fn init(dirs: &[&str], disable_system_mib: bool) -> Result<()> {
    let mut mib_data = Loader::new()
        .source(source::dirs(dirs).context("failed to load MIB files from directories")?);
    if !disable_system_mib {
        mib_data = mib_data.system_paths();
    };
    let model = mib_data.load().context("failed to load MIB data")?;
    if model.node_count() == 0 {
        // every OID name used in config will fail to resolve after this
        warn!(dirs = ?dirs, disable_system_mib, "no MIB files found");
    } else {
        debug!(nodes = model.node_count(), "MIB model loaded");
    }
    MODEL
        .set(model)
        .map_err(|_| anyhow!("MIB model has been set"))?;
    Ok(())
}

pub fn get_name_by_oid(oid: &Oid) -> Option<String> {
    let model = MODEL.get()?;
    Some(model.format_oid(oid))
}

pub fn get_oid_by_name(name: &str) -> Option<Oid> {
    let model = MODEL.get()?;
    model.resolve_oid(name).ok()
}

type CustomFormatterFn = fn(&Value) -> Option<payload::Value>;

struct CustomFormatter {
    formatter: Vec<(Vec<u32>, CustomFormatterFn)>,
    min_len: usize,
}
static PREDEFINED_FORMATTER: OnceLock<CustomFormatter> = OnceLock::new();

fn parse_as_path_segment(input: &[u8]) -> IResult<&[u8], payload::Value<'_>> {
    let (input, seg_type) = be_u8(input)?;
    let (input, seg_length) = be_u8(input)?;
    let (input, as_numbers) = count(be_u16, seg_length as usize).parse(input)?;

    let type_str = match seg_type {
        1 => "AS_SET",
        2 => "AS_SEQUENCE",
        _ => "UNKNOWN",
    };

    let fields = vec![
        (
            ZString::from_ref("type"),
            payload::Value::String(ZString::from_ref(type_str)),
        ),
        (
            ZString::from_ref("value"),
            payload::Value::Array(
                as_numbers
                    .into_iter()
                    .map(|n| payload::Value::Int64(n as i64))
                    .collect(),
            ),
        ),
    ];

    Ok((input, payload::Value::KvList(fields)))
}

fn as_path_formatter(value: &Value) -> Option<payload::Value<'_>> {
    let Value::OctetString(value) = value else {
        return None;
    };

    let (_, segments) = many0(parse_as_path_segment).parse(value).ok()?;

    Some(payload::Value::Array(segments))
}

fn ospf_fletcher_checksum(data: &[u8]) -> u16 {
    let mut c0: i32 = 0;
    let mut c1: i32 = 0;

    // OSPF Constants
    const CHECKSUM_OFFSET: usize = 16;
    let n = data.len() as i32;
    let l = (CHECKSUM_OFFSET + 1) as i32; // 1-indexed position (17)

    // 1. Calculate cumulative sums (skipping LS Age at bytes 0-1)
    for (i, &byte) in data.iter().enumerate().skip(2) {
        // Treat the checksum field itself (bytes 16 and 17) as zero,
        // but still accumulate c1 += c0 at every position.
        if i == CHECKSUM_OFFSET || i == CHECKSUM_OFFSET + 1 {
            c1 = (c1 + c0) % 255;
            continue;
        }
        c0 = (c0 + byte as i32) % 255;
        c1 = (c1 + c0) % 255;
    }

    // 2. Solve for X and Y to result in a zero checksum
    // Formula: X = ((n - L) * C0 - C1) % 255
    let mut x = ((n - l) * c0 - c1) % 255;
    if x <= 0 {
        x += 255;
    }

    // Formula: Y = ((L - 1 - n) * C0 + C1) % 255
    let mut y = ((l - 1 - n) * c0 + c1) % 255;
    if y <= 0 {
        y += 255;
    }

    ((x as u16) << 8) | (y as u16)
}

/// An LSA header's decoded fields, plus its LS type and checksum.
type LsaHeader<'a> = (Vec<(ZString<'a>, payload::Value<'a>)>, u8, u16);

fn parse_ospf_lsa_header(input: &[u8]) -> IResult<&[u8], LsaHeader<'_>> {
    let (input, ls_age) = be_u16(input)?;
    let (input, options) = be_u8(input)?;
    let (input, ls_type) = be_u8(input)?;
    let (input, link_state_id) = take(4usize)(input)?;
    let (input, advertising_router) = take(4usize)(input)?;
    let (input, ls_sequence_number) = be_i32(input)?;
    let (input, ls_checksum) = be_u16(input)?;
    let (input, length) = be_u16(input)?;

    let link_state_id: [u8; 4] = link_state_id.try_into().map_err(|_| {
        nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::LengthValue,
        ))
    })?;
    let advertising_router: [u8; 4] = advertising_router.try_into().map_err(|_| {
        nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::LengthValue,
        ))
    })?;

    let ls_type_str = match ls_type {
        1 => "Router-LSA",
        2 => "Network-LSA",
        3 => "Summary-LSA (IP network)",
        4 => "Summary-LSA (ASBR)",
        5 => "AS-external-LSA",
        _ => "Unknown",
    };

    let mut fields = Vec::with_capacity(9);
    fields.push((
        ZString::from_ref("ls_age"),
        payload::Value::Int64(ls_age as i64),
    ));
    fields.push((
        ZString::from_ref("options"),
        payload::Value::Int64(options as i64),
    ));
    fields.push((
        ZString::from_ref("ls_type"),
        payload::Value::String(ZString::from_ref(ls_type_str)),
    ));
    fields.push((
        ZString::from_ref("link_state_id"),
        payload::Value::String(ZString::from_smol(IpAddr::from(link_state_id).to_smolstr())),
    ));
    fields.push((
        ZString::from_ref("advertising_router"),
        payload::Value::String(ZString::from_smol(
            IpAddr::from(advertising_router).to_smolstr(),
        )),
    ));
    fields.push((
        ZString::from_ref("ls_sequence_number"),
        payload::Value::Int64(ls_sequence_number as i64),
    ));
    fields.push((
        ZString::from_ref("ls_checksum"),
        payload::Value::Int64(ls_checksum as i64),
    ));
    fields.push((
        ZString::from_ref("length"),
        payload::Value::Int64(length as i64),
    ));

    // Truncate input to LSA body length (total length - 20 byte header)
    // The length field includes the 20-byte header, so body is length - 20
    let body_length = length.saturating_sub(20) as usize;
    let (body_input, _rest) = if input.len() >= body_length {
        input.split_at(body_length)
    } else {
        // If not enough data, return what we have
        (input, &[][..])
    };

    Ok((body_input, (fields, ls_type, ls_checksum)))
}

fn parse_router_lsa(input: &[u8]) -> IResult<&[u8], Vec<(ZString<'_>, payload::Value<'_>)>> {
    let (input, flags) = be_u8(input)?;
    let (input, _reserved) = be_u8(input)?;
    let (input, num_links) = be_u16(input)?;

    let mut fields = Vec::with_capacity(2);

    // Parse router flags: bit 0 = V (Virtual link endpoint), bit 1 = E (AS boundary router), bit 2 = B (Area border router)
    let flag_fields = vec![
        (
            ZString::from_ref("V"),
            payload::Value::Bool((flags & 0x04) != 0),
        ),
        (
            ZString::from_ref("E"),
            payload::Value::Bool((flags & 0x02) != 0),
        ),
        (
            ZString::from_ref("B"),
            payload::Value::Bool((flags & 0x01) != 0),
        ),
    ];
    fields.push((
        ZString::from_ref("flags"),
        payload::Value::KvList(flag_fields),
    ));

    let mut links = Vec::new();
    let mut remaining = input;
    for _ in 0..num_links {
        let (input, link_id) = take(4usize)(remaining)?;
        let (input, link_data) = take(4usize)(input)?;
        let (input, link_type) = be_u8(input)?;
        let (input, num_tos) = be_u8(input)?;
        let (input, metric) = be_u16(input)?;

        let link_id: [u8; 4] = link_id.try_into().map_err(|_| {
            nom::Err::Error(nom::error::Error::new(
                remaining,
                nom::error::ErrorKind::LengthValue,
            ))
        })?;
        let link_data: [u8; 4] = link_data.try_into().map_err(|_| {
            nom::Err::Error(nom::error::Error::new(
                remaining,
                nom::error::ErrorKind::LengthValue,
            ))
        })?;

        let mut link_fields = Vec::with_capacity(5);
        link_fields.push((
            ZString::from_ref("link_id"),
            payload::Value::String(ZString::from_smol(IpAddr::from(link_id).to_smolstr())),
        ));
        link_fields.push((
            ZString::from_ref("link_data"),
            payload::Value::String(ZString::from_smol(IpAddr::from(link_data).to_smolstr())),
        ));
        let link_type_str = match link_type {
            1 => "Point-to-Point",
            2 => "Transit Network",
            3 => "Stub Network",
            4 => "Virtual Link",
            _ => "Unknown",
        };
        link_fields.push((
            ZString::from_ref("link_type"),
            payload::Value::String(ZString::from_ref(link_type_str)),
        ));
        link_fields.push((
            ZString::from_ref("metric"),
            payload::Value::Int64(metric as i64),
        ));

        // Parse TOS entries
        let mut tos_entries = Vec::new();
        remaining = input;
        for _ in 0..num_tos {
            let (input, tos) = be_u8(remaining)?;
            let (input, _reserved) = be_u8(input)?;
            let (input, tos_metric) = be_u16(input)?;

            let tos_fields = vec![
                (ZString::from_ref("tos"), payload::Value::Int64(tos as i64)),
                (
                    ZString::from_ref("metric"),
                    payload::Value::Int64(tos_metric as i64),
                ),
            ];

            tos_entries.push(payload::Value::KvList(tos_fields));

            remaining = input;
        }

        link_fields.push((ZString::from_ref("tos"), payload::Value::Array(tos_entries)));

        links.push(payload::Value::KvList(link_fields));
    }

    fields.push((ZString::from_ref("links"), payload::Value::Array(links)));

    Ok((remaining, fields))
}

fn parse_network_lsa(input: &[u8]) -> IResult<&[u8], Vec<(ZString<'_>, payload::Value<'_>)>> {
    let (input, network_mask) = take(4usize)(input)?;
    let network_mask: [u8; 4] = network_mask.try_into().map_err(|_| {
        nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::LengthValue,
        ))
    })?;

    let mut routers = Vec::new();
    let mut remaining = input;
    while remaining.len() >= 4 {
        let (input, router_id) = take(4usize)(remaining)?;
        let router_id: [u8; 4] = router_id.try_into().map_err(|_| {
            nom::Err::Error(nom::error::Error::new(
                remaining,
                nom::error::ErrorKind::LengthValue,
            ))
        })?;
        routers.push(payload::Value::String(ZString::from_smol(
            IpAddr::from(router_id).to_smolstr(),
        )));
        remaining = input;
    }

    let fields = vec![
        (
            ZString::from_ref("network_mask"),
            payload::Value::String(ZString::from_smol(IpAddr::from(network_mask).to_smolstr())),
        ),
        (
            ZString::from_ref("attached_routers"),
            payload::Value::Array(routers),
        ),
    ];

    Ok((remaining, fields))
}

fn parse_summary_lsa(input: &[u8]) -> IResult<&[u8], Vec<(ZString<'_>, payload::Value<'_>)>> {
    let (input, network_mask) = take(4usize)(input)?;
    let (input, _reserved) = be_u8(input)?;
    let (input, metric) = be_u8(input)?;
    let (input, metric2) = be_u16(input)?;

    let network_mask: [u8; 4] = network_mask.try_into().map_err(|_| {
        nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::LengthValue,
        ))
    })?;
    let metric = ((metric as u32) << 16) | (metric2 as u32);

    let mut fields = Vec::with_capacity(3);
    fields.push((
        ZString::from_ref("network_mask"),
        payload::Value::String(ZString::from_smol(IpAddr::from(network_mask).to_smolstr())),
    ));
    fields.push((
        ZString::from_ref("metric"),
        payload::Value::Int64(metric as i64),
    ));

    // Parse TOS entries
    let mut tos_entries = Vec::new();
    let mut remaining = input;
    while remaining.len() >= 4 {
        let (input, tos) = be_u8(remaining)?;
        let (input, metric_byte1) = be_u8(input)?;
        let (input, metric_byte2) = be_u16(input)?;

        let tos_metric = ((metric_byte1 as u32) << 16) | (metric_byte2 as u32);

        let tos_fields = vec![
            (ZString::from_ref("tos"), payload::Value::Int64(tos as i64)),
            (
                ZString::from_ref("metric"),
                payload::Value::Int64(tos_metric as i64),
            ),
        ];

        tos_entries.push(payload::Value::KvList(tos_fields));

        remaining = input;
    }

    fields.push((ZString::from_ref("tos"), payload::Value::Array(tos_entries)));

    Ok((remaining, fields))
}

fn parse_as_external_lsa(input: &[u8]) -> IResult<&[u8], Vec<(ZString<'_>, payload::Value<'_>)>> {
    let (input, network_mask) = take(4usize)(input)?;
    let (input, e_bit_and_metric) = be_u8(input)?;
    let (input, metric2) = be_u8(input)?;
    let (input, metric3) = be_u16(input)?;
    let (input, forwarding_address) = take(4usize)(input)?;
    let (input, external_route_tag) = be_u32(input)?;

    let network_mask: [u8; 4] = network_mask.try_into().map_err(|_| {
        nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::LengthValue,
        ))
    })?;
    let forwarding_address: [u8; 4] = forwarding_address.try_into().map_err(|_| {
        nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::LengthValue,
        ))
    })?;
    let e_bit = (e_bit_and_metric & 0x80) != 0;
    let metric =
        (((e_bit_and_metric & 0x7F) as u32) << 16) | ((metric2 as u32) << 8) | (metric3 as u32);

    let mut fields = Vec::with_capacity(6);
    fields.push((
        ZString::from_ref("network_mask"),
        payload::Value::String(ZString::from_smol(IpAddr::from(network_mask).to_smolstr())),
    ));
    fields.push((
        ZString::from_ref("external_type"),
        payload::Value::String(ZString::from_ref(if e_bit { "Type 2" } else { "Type 1" })),
    ));
    fields.push((
        ZString::from_ref("metric"),
        payload::Value::Int64(metric as i64),
    ));
    fields.push((
        ZString::from_ref("forwarding_address"),
        payload::Value::String(ZString::from_smol(
            IpAddr::from(forwarding_address).to_smolstr(),
        )),
    ));
    fields.push((
        ZString::from_ref("external_route_tag"),
        payload::Value::Int64(external_route_tag as i64),
    ));

    // Parse TOS entries
    let mut tos_entries = Vec::new();
    let mut remaining = input;
    while remaining.len() >= 12 {
        let (input, e_bit_and_tos_metric) = be_u8(remaining)?;
        let (input, metric_byte2) = be_u8(input)?;
        let (input, metric_byte3) = be_u16(input)?;
        let (input, tos_forwarding_address) = take(4usize)(input)?;
        let (input, tos_external_route_tag) = be_u32(input)?;

        let tos_forwarding_address: [u8; 4] = tos_forwarding_address.try_into().map_err(|_| {
            nom::Err::Error(nom::error::Error::new(
                remaining,
                nom::error::ErrorKind::LengthValue,
            ))
        })?;

        let tos = e_bit_and_tos_metric & 0x7F;
        let tos_e_bit = (e_bit_and_tos_metric & 0x80) != 0;
        let tos_metric = ((metric_byte2 as u32) << 8) | (metric_byte3 as u32);

        let mut tos_fields = Vec::with_capacity(5);
        tos_fields.push((ZString::from_ref("tos"), payload::Value::Int64(tos as i64)));
        tos_fields.push((
            ZString::from_ref("external_type"),
            payload::Value::String(ZString::from_ref(if tos_e_bit {
                "Type 2"
            } else {
                "Type 1"
            })),
        ));
        tos_fields.push((
            ZString::from_ref("metric"),
            payload::Value::Int64(tos_metric as i64),
        ));
        tos_fields.push((
            ZString::from_ref("forwarding_address"),
            payload::Value::String(ZString::from_smol(
                IpAddr::from(tos_forwarding_address).to_smolstr(),
            )),
        ));
        tos_fields.push((
            ZString::from_ref("external_route_tag"),
            payload::Value::Int64(tos_external_route_tag as i64),
        ));

        tos_entries.push(payload::Value::KvList(tos_fields));

        remaining = input;
    }

    fields.push((ZString::from_ref("tos"), payload::Value::Array(tos_entries)));

    Ok((remaining, fields))
}

fn ospf_lsdb_advertisement_formatter(value: &Value) -> Option<payload::Value<'_>> {
    let Value::OctetString(value) = value else {
        return None;
    };

    let (input, (mut header_fields, ls_type, ls_checksum)) = parse_ospf_lsa_header(value).ok()?;

    // Calculate and verify checksum
    let calculated_checksum = ospf_fletcher_checksum(value);
    let checksum_valid = calculated_checksum == ls_checksum;

    // Add checksum_valid field to header
    header_fields.push((
        ZString::from_ref("checksum_valid"),
        payload::Value::Bool(checksum_valid),
    ));

    let mut fields = Vec::with_capacity(2);
    fields.push((
        ZString::from_ref("header"),
        payload::Value::KvList(header_fields),
    ));

    let value_result = match ls_type {
        1 => parse_router_lsa(input).ok(),
        2 => parse_network_lsa(input).ok(),
        3 | 4 => parse_summary_lsa(input).ok(),
        5 => parse_as_external_lsa(input).ok(),
        _ => None,
    };

    if let Some((_, value_fields)) = value_result {
        fields.push((
            ZString::from_ref("value"),
            payload::Value::KvList(value_fields),
        ));
    }

    Some(payload::Value::KvList(fields))
}

fn predefined_formatter<'a>(oid: &[u32], value: &'a Value) -> Option<payload::Value<'a>> {
    let formatter = PREDEFINED_FORMATTER.get_or_init(|| {
        let mut cf = CustomFormatter {
            formatter: Vec::default(),
            min_len: usize::MAX,
        };
        // BGP4-MIB::bgp4PathAttrASPathSegment
        let oid: Vec<u32> = vec![1, 3, 6, 1, 2, 1, 15, 6, 1, 5];
        cf.min_len = cf.min_len.min(oid.len());
        cf.formatter.push((oid, as_path_formatter));
        // OSPF-MIB::ospfLsdbAdvertisement
        let oid: Vec<u32> = vec![1, 3, 6, 1, 2, 1, 14, 4, 1, 8];
        cf.min_len = cf.min_len.min(oid.len());
        cf.formatter.push((oid, ospf_lsdb_advertisement_formatter));
        cf
    });

    if oid.len() < formatter.min_len {
        return None;
    }

    // Try to find a formatter by matching the prefix of the OID
    for (prefix, func) in &formatter.formatter {
        if oid.starts_with(prefix) {
            return func(value);
        }
    }
    None
}

pub fn format_value<'a>(oid: &Oid, value: &'a Value) -> Option<payload::Value<'a>> {
    if let Some(formatted) = predefined_formatter(oid, value) {
        return Some(formatted);
    }
    let model = MODEL.get()?;
    let node = model.lookup_oid(oid);
    let object = node.object()?;
    if let (enum_, Value::Integer(value)) = (object.effective_enums(), value)
        && let Some(enum_) = enum_.iter().find(|e| e.value == *value as i64)
    {
        return Some(payload::Value::String(ZString::from_ref(&enum_.label)));
    }
    let hint = object.effective_display_hint();
    value
        .format_with_hint(hint)
        .map(|v| payload::Value::String(ZString::from_owned(v)))
}

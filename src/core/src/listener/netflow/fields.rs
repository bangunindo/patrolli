// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use netflow_parser::V9Field;
use netflow_parser::protocol::ProtocolTypes;
use netflow_parser::variable_versions::ipfix::lookup::{
    CiscoIPFixField, IANAIPFixField, NatIPFixField, NetscalerIPFixField, ReverseInformationElement,
    VMWareIPFixField, YafIPFixField,
};

pub fn prototype_field_map(field: &ProtocolTypes) -> Option<&'static str> {
    match field {
        ProtocolTypes::Hopopt => Some("hopopt"),
        ProtocolTypes::Icmp => Some("icmp"),
        ProtocolTypes::Igmp => Some("igmp"),
        ProtocolTypes::Ggp => Some("ggp"),
        ProtocolTypes::Ipv4 => Some("ipv4"),
        ProtocolTypes::St => Some("st"),
        ProtocolTypes::Tcp => Some("tcp"),
        ProtocolTypes::Cbt => Some("cbt"),
        ProtocolTypes::Egp => Some("egp"),
        ProtocolTypes::Igp => Some("igp"),
        ProtocolTypes::Bbcrccmon => Some("bbcrccmon"),
        ProtocolTypes::Nvpii => Some("nvpii"),
        ProtocolTypes::Pup => Some("pup"),
        ProtocolTypes::Argus => Some("argus"),
        ProtocolTypes::Emcon => Some("emcon"),
        ProtocolTypes::Xnet => Some("xnet"),
        ProtocolTypes::Chaos => Some("chaos"),
        ProtocolTypes::Udp => Some("udp"),
        ProtocolTypes::Mux => Some("mux"),
        ProtocolTypes::Dcnmeas => Some("dcnmeas"),
        ProtocolTypes::Hmp => Some("hmp"),
        ProtocolTypes::Prm => Some("prm"),
        ProtocolTypes::Xnxidp => Some("xnxidp"),
        ProtocolTypes::Trunk1 => Some("trunk1"),
        ProtocolTypes::Trunk2 => Some("trunk2"),
        ProtocolTypes::Leaf1 => Some("leaf1"),
        ProtocolTypes::Leaf2 => Some("leaf2"),
        ProtocolTypes::Rdp => Some("rdp"),
        ProtocolTypes::Irtp => Some("irtp"),
        ProtocolTypes::Isotp4 => Some("isotp4"),
        ProtocolTypes::Netblt => Some("netblt"),
        ProtocolTypes::Mfensp => Some("mfensp"),
        ProtocolTypes::Meritinp => Some("meritinp"),
        ProtocolTypes::Dccp => Some("dccp"),
        ProtocolTypes::Threepc => Some("threepc"),
        ProtocolTypes::Idpr => Some("idpr"),
        ProtocolTypes::Xtp => Some("xtp"),
        ProtocolTypes::Ddp => Some("ddp"),
        ProtocolTypes::Idprcmtp => Some("idprcmtp"),
        ProtocolTypes::Tppp => Some("tppp"),
        ProtocolTypes::Il => Some("il"),
        ProtocolTypes::Ipv6 => Some("ipv6"),
        ProtocolTypes::Sdrp => Some("sdrp"),
        ProtocolTypes::Ipv6Route => Some("ipv6route"),
        ProtocolTypes::Ipv6Frag => Some("ipv6frag"),
        ProtocolTypes::Idrp => Some("idrp"),
        ProtocolTypes::Rsvp => Some("rsvp"),
        ProtocolTypes::Gre => Some("gre"),
        ProtocolTypes::Dsr => Some("dsr"),
        ProtocolTypes::Bna => Some("bna"),
        ProtocolTypes::Esp => Some("esp"),
        ProtocolTypes::Ah => Some("ah"),
        ProtocolTypes::Inlsp => Some("inlsp"),
        ProtocolTypes::Swipe => Some("swipe"),
        ProtocolTypes::Narp => Some("narp"),
        ProtocolTypes::Mobile => Some("mobile"),
        ProtocolTypes::Tlsp => Some("tlsp"),
        ProtocolTypes::Skip => Some("skip"),
        ProtocolTypes::Ipv6Icmp => Some("ipv6icmp"),
        ProtocolTypes::Ipv6Nonxt => Some("ipv6nonxt"),
        ProtocolTypes::Ipv6Opts => Some("ipv6opts"),
        ProtocolTypes::Anydistributedprotocol => Some("anydistributedprotocol"),
        ProtocolTypes::Cftp => Some("cftp"),
        ProtocolTypes::Anylocalnetwork => Some("anylocalnetwork"),
        ProtocolTypes::Satexpak => Some("satexpak"),
        ProtocolTypes::Kryptolan => Some("kryptolan"),
        ProtocolTypes::Rvd => Some("rvd"),
        ProtocolTypes::Ippc => Some("ippc"),
        ProtocolTypes::Anydistributedfilesystem => Some("anydistributedfilesystem"),
        ProtocolTypes::Satmon => Some("satmon"),
        ProtocolTypes::Visa => Some("visa"),
        ProtocolTypes::Ipcv => Some("ipcv"),
        ProtocolTypes::Cpnx => Some("cpnx"),
        ProtocolTypes::Cphb => Some("cphb"),
        ProtocolTypes::Wsn => Some("wsn"),
        ProtocolTypes::Pvp => Some("pvp"),
        ProtocolTypes::Brsatmon => Some("brsatmon"),
        ProtocolTypes::Sunnd => Some("sunnd"),
        ProtocolTypes::Wbmon => Some("wbmon"),
        ProtocolTypes::Wbexpak => Some("wbexpak"),
        ProtocolTypes::Isoip => Some("isoip"),
        ProtocolTypes::Vmtp => Some("vmtp"),
        ProtocolTypes::Securevmtp => Some("securevmtp"),
        ProtocolTypes::Vines => Some("vines"),
        ProtocolTypes::Iptm => Some("iptm"),
        ProtocolTypes::Nsfnetigp => Some("nsfnetigp"),
        ProtocolTypes::Dgp => Some("dgp"),
        ProtocolTypes::Tcf => Some("tcf"),
        ProtocolTypes::Eigrp => Some("eigrp"),
        ProtocolTypes::Ospfigp => Some("ospfigp"),
        ProtocolTypes::Spriterpc => Some("spriterpc"),
        ProtocolTypes::Larp => Some("larp"),
        ProtocolTypes::Mtp => Some("mtp"),
        ProtocolTypes::Ax25 => Some("ax25"),
        ProtocolTypes::Ipip => Some("ipip"),
        ProtocolTypes::Micp => Some("micp"),
        ProtocolTypes::Sccsp => Some("sccsp"),
        ProtocolTypes::Etherip => Some("etherip"),
        ProtocolTypes::Encap => Some("encap"),
        ProtocolTypes::Anyprivateencryptionscheme => Some("anyprivateencryptionscheme"),
        ProtocolTypes::Gmtp => Some("gmtp"),
        ProtocolTypes::Ifmp => Some("ifmp"),
        ProtocolTypes::Pnni => Some("pnni"),
        ProtocolTypes::Pim => Some("pim"),
        ProtocolTypes::Aris => Some("aris"),
        ProtocolTypes::Scps => Some("scps"),
        ProtocolTypes::Qnx => Some("qnx"),
        ProtocolTypes::An => Some("an"),
        ProtocolTypes::Ipcomp => Some("ipcomp"),
        ProtocolTypes::Snp => Some("snp"),
        ProtocolTypes::Compaqpeer => Some("compaqpeer"),
        ProtocolTypes::Ipxinip => Some("ipxinip"),
        ProtocolTypes::Vrrp => Some("vrrp"),
        ProtocolTypes::Pgm => Some("pgm"),
        ProtocolTypes::Any0Hopprotocol => Some("any0hopprotocol"),
        ProtocolTypes::L2Tp => Some("l2tp"),
        ProtocolTypes::Ddx => Some("ddx"),
        ProtocolTypes::Iatp => Some("iatp"),
        ProtocolTypes::Stp => Some("stp"),
        ProtocolTypes::Srp => Some("srp"),
        ProtocolTypes::Uti => Some("uti"),
        ProtocolTypes::Smp => Some("smp"),
        ProtocolTypes::Sm => Some("sm"),
        ProtocolTypes::Ptp => Some("ptp"),
        ProtocolTypes::Isisoveripv4 => Some("isisoveripv4"),
        ProtocolTypes::Fire => Some("fire"),
        ProtocolTypes::Crtp => Some("crtp"),
        ProtocolTypes::Crudp => Some("crudp"),
        ProtocolTypes::Sscopmce => Some("sscopmce"),
        ProtocolTypes::Iplt => Some("iplt"),
        ProtocolTypes::Sps => Some("sps"),
        ProtocolTypes::Pipe => Some("pipe"),
        ProtocolTypes::Sctp => Some("sctp"),
        ProtocolTypes::Fc => Some("fc"),
        ProtocolTypes::Rsvpe2Eignore => Some("rsvpe2eignore"),
        ProtocolTypes::Mobilityheader => Some("mobilityheader"),
        ProtocolTypes::Udplite => Some("udplite"),
        ProtocolTypes::Mplsinip => Some("mplsinip"),
        ProtocolTypes::Manet => Some("manet"),
        ProtocolTypes::Hip => Some("hip"),
        ProtocolTypes::Shim6 => Some("shim6"),
        ProtocolTypes::Wesp => Some("wesp"),
        ProtocolTypes::Rohc => Some("rohc"),
        ProtocolTypes::Ethernet => Some("ethernet"),
        ProtocolTypes::Aggfrag => Some("aggfrag"),
        ProtocolTypes::Reserved => Some("reserved"),
        _ => None,
    }
}

pub fn v9_field_map(field: &V9Field) -> Result<&'static str, u16> {
    match field {
        V9Field::InBytes => Ok("nf.data.IN_BYTES"),
        V9Field::InPkts => Ok("nf.data.IN_PKTS"),
        V9Field::Flows => Ok("nf.data.FLOWS"),
        V9Field::Protocol => Ok("nf.data.PROTOCOL"),
        V9Field::SrcTos => Ok("nf.data.SRC_TOS"),
        V9Field::TcpFlags => Ok("nf.data.TCP_FLAGS"),
        V9Field::L4SrcPort => Ok("nf.data.L4_SRC_PORT"),
        V9Field::Ipv4SrcAddr => Ok("nf.data.IPV4_SRC_ADDR"),
        V9Field::SrcMask => Ok("nf.data.SRC_MASK"),
        V9Field::InputSnmp => Ok("nf.data.INPUT_SNMP"),
        V9Field::L4DstPort => Ok("nf.data.L4_DST_PORT"),
        V9Field::Ipv4DstAddr => Ok("nf.data.IPV4_DST_ADDR"),
        V9Field::DstMask => Ok("nf.data.DST_MASK"),
        V9Field::OutputSnmp => Ok("nf.data.OUTPUT_SNMP"),
        V9Field::Ipv4NextHop => Ok("nf.data.IPV4_NEXT_HOP"),
        V9Field::SrcAs => Ok("nf.data.SRC_AS"),
        V9Field::DstAs => Ok("nf.data.DST_AS"),
        V9Field::BgpIpv4NextHop => Ok("nf.data.BGP_IPV4_NEXT_HOP"),
        V9Field::MulDstPkts => Ok("nf.data.MUL_DST_PKTS"),
        V9Field::MulDstBytes => Ok("nf.data.MUL_DST_BYTES"),
        V9Field::LastSwitched => Ok("nf.data.LAST_SWITCHED"),
        V9Field::FirstSwitched => Ok("nf.data.FIRST_SWITCHED"),
        V9Field::OutBytes => Ok("nf.data.OUT_BYTES"),
        V9Field::OutPkts => Ok("nf.data.OUT_PKTS"),
        V9Field::MinPktLngth => Ok("nf.data.MIN_PKT_LNGTH"),
        V9Field::MaxPktLngth => Ok("nf.data.MAX_PKT_LNGTH"),
        V9Field::Ipv6SrcAddr => Ok("nf.data.IPV6_SRC_ADDR"),
        V9Field::Ipv6DstAddr => Ok("nf.data.IPV6_DST_ADDR"),
        V9Field::Ipv6SrcMask => Ok("nf.data.IPV6_SRC_MASK"),
        V9Field::Ipv6DstMask => Ok("nf.data.IPV6_DST_MASK"),
        V9Field::Ipv6FlowLabel => Ok("nf.data.IPV6_FLOW_LABEL"),
        V9Field::IcmpType => Ok("nf.data.ICMP_TYPE"),
        V9Field::MulIgmpType => Ok("nf.data.MUL_IGMP_TYPE"),
        V9Field::SamplingInterval => Ok("nf.data.SAMPLING_INTERVAL"),
        V9Field::SamplingAlgorithm => Ok("nf.data.SAMPLING_ALGORITHM"),
        V9Field::FlowActiveTimeout => Ok("nf.data.FLOW_ACTIVE_TIMEOUT"),
        V9Field::FlowInactiveTimeout => Ok("nf.data.FLOW_INACTIVE_TIMEOUT"),
        V9Field::EngineType => Ok("nf.data.ENGINE_TYPE"),
        V9Field::EngineId => Ok("nf.data.ENGINE_ID"),
        V9Field::TotalBytesExp => Ok("nf.data.TOTAL_BYTES_EXP"),
        V9Field::TotalPktsExp => Ok("nf.data.TOTAL_PKTS_EXP"),
        V9Field::TotalFlowsExp => Ok("nf.data.TOTAL_FLOWS_EXP"),
        V9Field::Ipv4SrcPrefix => Ok("nf.data.IPV4_SRC_PREFIX"),
        V9Field::Ipv4DstPrefix => Ok("nf.data.IPV4_DST_PREFIX"),
        V9Field::MplsTopLabelType => Ok("nf.data.MPLS_TOP_LABEL_TYPE"),
        V9Field::MplsTopLabelIpAddr => Ok("nf.data.MPLS_TOP_LABEL_IP_ADDR"),
        V9Field::FlowSamplerId => Ok("nf.data.FLOW_SAMPLER_ID"),
        V9Field::FlowSamplerMode => Ok("nf.data.FLOW_SAMPLER_MODE"),
        V9Field::FlowSamplerRandomInterval => Ok("nf.data.FLOW_SAMPLER_RANDOM_INTERVAL"),
        V9Field::MinTtl => Ok("nf.data.MIN_TTL"),
        V9Field::MaxTtl => Ok("nf.data.MAX_TTL"),
        V9Field::Ipv4Ident => Ok("nf.data.IPV4_IDENT"),
        V9Field::DstTos => Ok("nf.data.DST_TOS"),
        V9Field::InSrcMac => Ok("nf.data.IN_SRC_MAC"),
        V9Field::OutDstMac => Ok("nf.data.OUT_DST_MAC"),
        V9Field::SrcVlan => Ok("nf.data.SRC_VLAN"),
        V9Field::DstVlan => Ok("nf.data.DST_VLAN"),
        V9Field::IpProtocolVersion => Ok("nf.data.IP_PROTOCOL_VERSION"),
        V9Field::Direction => Ok("nf.data.DIRECTION"),
        V9Field::Ipv6NextHop => Ok("nf.data.IPV6_NEXT_HOP"),
        V9Field::BgpIpv6NextHop => Ok("nf.data.BGP_IPV6_NEXT_HOP"),
        V9Field::Ipv6OptionHeaders => Ok("nf.data.IPV6_OPTION_HEADERS"),
        V9Field::MplsLabel1 => Ok("nf.data.MPLS_LABEL_1"),
        V9Field::MplsLabel2 => Ok("nf.data.MPLS_LABEL_2"),
        V9Field::MplsLabel3 => Ok("nf.data.MPLS_LABEL_3"),
        V9Field::MplsLabel4 => Ok("nf.data.MPLS_LABEL_4"),
        V9Field::MplsLabel5 => Ok("nf.data.MPLS_LABEL_5"),
        V9Field::MplsLabel6 => Ok("nf.data.MPLS_LABEL_6"),
        V9Field::MplsLabel7 => Ok("nf.data.MPLS_LABEL_7"),
        V9Field::MplsLabel8 => Ok("nf.data.MPLS_LABEL_8"),
        V9Field::MplsLabel9 => Ok("nf.data.MPLS_LABEL_9"),
        V9Field::MplsLabel10 => Ok("nf.data.MPLS_LABEL_10"),
        V9Field::InDstMac => Ok("nf.data.IN_DST_MAC"),
        V9Field::OutSrcMac => Ok("nf.data.OUT_SRC_MAC"),
        V9Field::IfName => Ok("nf.data.IF_NAME"),
        V9Field::IfDesc => Ok("nf.data.IF_DESC"),
        V9Field::SamplerName => Ok("nf.data.SAMPLER_NAME"),
        V9Field::InPermanentBytes => Ok("nf.data.IN_PERMANENT_BYTES"),
        V9Field::InPermanentPkts => Ok("nf.data.IN_PERMANENT_PKTS"),
        V9Field::FragmentOffset => Ok("nf.data.FRAGMENT_OFFSET"),
        V9Field::ForwardingStatus => Ok("nf.data.FORWARDING_STATUS"),
        V9Field::MplsPalRd => Ok("nf.data.MPLS_PAL_RD"),
        V9Field::MplsPrefixLen => Ok("nf.data.MPLS_PREFIX_LEN"),
        V9Field::SrcTrafficIndex => Ok("nf.data.SRC_TRAFFIC_INDEX"),
        V9Field::DstTrafficIndex => Ok("nf.data.DST_TRAFFIC_INDEX"),
        V9Field::ApplicationDescription => Ok("nf.data.APPLICATION_DESCRIPTION"),
        V9Field::ApplicationTag => Ok("nf.data.APPLICATION_TAG"),
        V9Field::ApplicationName => Ok("nf.data.APPLICATION_NAME"),
        V9Field::PostipDiffServCodePoint => Ok("nf.data.POST_IP_DIFF_SERV_CODE_POINT"),
        V9Field::Replicationfactor => Ok("nf.data.REPLICATION_FACTOR"),
        V9Field::Deprecated => Ok("nf.data.DEPRECATED"),
        V9Field::Layer2packetSectionOffset => Ok("nf.data.LAYER2_PACKET_SECTION_OFFSET"),
        V9Field::Layer2packetSectionSize => Ok("nf.data.LAYER2_PACKET_SECTION_SIZE"),
        V9Field::Layer2packetSectionData => Ok("nf.data.LAYER2_PACKET_SECTION_DATA"),
        V9Field::BgpNextAdjacentAsNumber => Ok("nf.data.BGP_NEXT_ADJACENT_AS_NUMBER"),
        V9Field::BgpPrevAdjacentAsNumber => Ok("nf.data.BGP_PREV_ADJACENT_AS_NUMBER"),
        V9Field::ExporterIpv4Address => Ok("nf.data.EXPORTER_IPV4_ADDRESS"),
        V9Field::ExporterIpv6Address => Ok("nf.data.EXPORTER_IPV6_ADDRESS"),
        V9Field::DroppedOctetDeltaCount => Ok("nf.data.DROPPED_OCTET_DELTA_COUNT"),
        V9Field::DroppedPacketDeltaCount => Ok("nf.data.DROPPED_PACKET_DELTA_COUNT"),
        V9Field::DroppedOctetTotalCount => Ok("nf.data.DROPPED_OCTET_TOTAL_COUNT"),
        V9Field::DroppedPacketTotalCount => Ok("nf.data.DROPPED_PACKET_TOTAL_COUNT"),
        V9Field::FlowEndReason => Ok("nf.data.FLOW_END_REASON"),
        V9Field::CommonPropertiesId => Ok("nf.data.COMMON_PROPERTIES_ID"),
        V9Field::ObservationPointId => Ok("nf.data.OBSERVATION_POINT_ID"),
        V9Field::IcmpTypeCodeIpv6 => Ok("nf.data.ICMP_TYPE_CODE_IPV6"),
        V9Field::MplsTopLabelIpv6Address => Ok("nf.data.MPLS_TOP_LABEL_IPV6_ADDRESS"),
        V9Field::LineCardId => Ok("nf.data.LINE_CARD_ID"),
        V9Field::PortId => Ok("nf.data.PORT_ID"),
        V9Field::MeteringProcessId => Ok("nf.data.METERING_PROCESS_ID"),
        V9Field::ExportingProcessId => Ok("nf.data.EXPORTING_PROCESS_ID"),
        V9Field::TemplateId => Ok("nf.data.TEMPLATE_ID"),
        V9Field::WlanChannelId => Ok("nf.data.WLAN_CHANNEL_ID"),
        V9Field::WlanSsid => Ok("nf.data.WLAN_SSID"),
        V9Field::FlowId => Ok("nf.data.FLOW_ID"),
        V9Field::ObservationDomainId => Ok("nf.data.OBSERVATION_DOMAIN_ID"),
        V9Field::FlowStartSeconds => Ok("nf.data.FLOW_START_SECONDS"),
        V9Field::FlowEndSeconds => Ok("nf.data.FLOW_END_SECONDS"),
        V9Field::FlowStartMilliseconds => Ok("nf.data.FLOW_START_MILLISECONDS"),
        V9Field::FlowEndMilliseconds => Ok("nf.data.FLOW_END_MILLISECONDS"),
        V9Field::FlowStartMicroseconds => Ok("nf.data.FLOW_START_MICROSECONDS"),
        V9Field::FlowEndMicroseconds => Ok("nf.data.FLOW_END_MICROSECONDS"),
        V9Field::FlowStartNanoseconds => Ok("nf.data.FLOW_START_NANOSECONDS"),
        V9Field::FlowEndNanoseconds => Ok("nf.data.FLOW_END_NANOSECONDS"),
        V9Field::FlowStartDeltaMicroseconds => Ok("nf.data.FLOW_START_DELTA_MICROSECONDS"),
        V9Field::FlowEndDeltaMicroseconds => Ok("nf.data.FLOW_END_DELTA_MICROSECONDS"),
        V9Field::SystemInitTimeMilliseconds => Ok("nf.data.SYSTEM_INIT_TIME_MILLISECONDS"),
        V9Field::FlowDurationMilliseconds => Ok("nf.data.FLOW_DURATION_MILLISECONDS"),
        V9Field::FlowDurationMicroseconds => Ok("nf.data.FLOW_DURATION_MICROSECONDS"),
        V9Field::ObservedFlowTotalCount => Ok("nf.data.OBSERVED_FLOW_TOTAL_COUNT"),
        V9Field::IgnoredPacketTotalCount => Ok("nf.data.IGNORED_PACKET_TOTAL_COUNT"),
        V9Field::IgnoredOctetTotalCount => Ok("nf.data.IGNORED_OCTET_TOTAL_COUNT"),
        V9Field::NotSentFlowTotalCount => Ok("nf.data.NOT_SENT_FLOW_TOTAL_COUNT"),
        V9Field::NotSentPacketTotalCount => Ok("nf.data.NOT_SENT_PACKET_TOTAL_COUNT"),
        V9Field::NotSentOctetTotalCount => Ok("nf.data.NOT_SENT_OCTET_TOTAL_COUNT"),
        V9Field::DestinationIpv6Prefix => Ok("nf.data.DESTINATION_IPV6_PREFIX"),
        V9Field::SourceIpv6Prefix => Ok("nf.data.SOURCE_IPV6_PREFIX"),
        V9Field::PostOctetTotalCount => Ok("nf.data.POST_OCTET_TOTAL_COUNT"),
        V9Field::PostPacketTotalCount => Ok("nf.data.POST_PACKET_TOTAL_COUNT"),
        V9Field::FlowKeyIndicator => Ok("nf.data.FLOW_KEY_INDICATOR"),
        V9Field::PostMcastPacketTotalCount => Ok("nf.data.POST_MCAST_PACKET_TOTAL_COUNT"),
        V9Field::PostMcastOctetTotalCount => Ok("nf.data.POST_MCAST_OCTET_TOTAL_COUNT"),
        V9Field::IcmpTypeValue => Ok("nf.data.ICMP_TYPE_VALUE"),
        V9Field::IcmpCodeValue => Ok("nf.data.ICMP_CODE_VALUE"),
        V9Field::IcmpIpv6TypeValue => Ok("nf.data.ICMP_IPV6_TYPE_VALUE"),
        V9Field::IcmpIpv6CodeValue => Ok("nf.data.ICMP_IPV6_CODE_VALUE"),
        V9Field::PostNATSourceIPv4Address => Ok("nf.data.POST_NAT_SOURCE_IPV4_ADDRESS"),
        V9Field::PostNATDestinationIPv4Address => Ok("nf.data.POST_NAT_DESTINATION_IPV4_ADDRESS"),
        V9Field::PostNATTSourceTransportPort => Ok("nf.data.POST_NAPT_SOURCE_TRANSPORT_PORT"),
        V9Field::PostNATTDestinationTransportPort => {
            Ok("nf.data.POST_NAPT_DESTINATION_TRANSPORT_PORT")
        }
        V9Field::NatOriginatingAddressRealm => Ok("nf.data.NAT_ORIGINATING_ADDRESS_REALM"),
        V9Field::NatEvent => Ok("nf.data.NAT_EVENT"),
        V9Field::PostNATSourceIpv6Address => Ok("nf.data.POST_NAT_SOURCE_IPV6_ADDRESS"),
        V9Field::PostNATDestinationIpv6Address => Ok("nf.data.POST_NAT_DESTINATION_IPV6_ADDRESS"),
        V9Field::FirewallEvent => Ok("nf.data.FIREWALL_EVENT"),
        V9Field::IngressVRFID => Ok("nf.data.INGRESS_VRF_ID"),
        V9Field::EgressVRFID => Ok("nf.data.EGRESS_VRF_ID"),
        V9Field::BiflowDirection => Ok("nf.data.BIFLOW_DIRECTION"),
        V9Field::ObservationTimeMilliseconds => Ok("nf.data.OBSERVATION_TIME_MILLISECONDS"),
        V9Field::IanaAssigned(num) => {
            let iana = IANAIPFixField::from(*num);
            ipfix_iana_field_map(&iana)
        }
        V9Field::Unknown(num) => Err(*num),
    }
}

pub fn ipfix_cisco_field_map(field: &CiscoIPFixField) -> Result<&'static str, u16> {
    {
        match field {
            CiscoIPFixField::CiscoServerBytesNetwork => Ok("nf.data.ciscoServerBytesNetwork"),
            CiscoIPFixField::CiscoClientBytesNetwork => Ok("nf.data.ciscoClientBytesNetwork"),
            CiscoIPFixField::CiscoServicesWaasSegment => Ok("nf.data.ciscoServicesWaasSegment"),
            CiscoIPFixField::CiscoServicesWaasPassthroughReason => {
                Ok("nf.data.ciscoServicesWaasPassthroughReason")
            }
            CiscoIPFixField::CiscoAppHttpUriStatistics => Ok("nf.data.ciscoAppHttpUriStatistics"),
            CiscoIPFixField::CiscoAppCategoryName => Ok("nf.data.ciscoAppCategoryName"),
            CiscoIPFixField::CiscoAppGroupName => Ok("nf.data.ciscoAppGroupName"),
            CiscoIPFixField::CiscoAppHttpHost => Ok("nf.data.ciscoAppHttpHost"),
            CiscoIPFixField::CiscoClientIpv4Address => Ok("nf.data.ciscoClientIpv4Address"),
            CiscoIPFixField::CiscoServerIpv4Address => Ok("nf.data.ciscoServerIpv4Address"),
            CiscoIPFixField::CiscoClientL4Port => Ok("nf.data.ciscoClientL4Port"),
            CiscoIPFixField::CiscoServerL4Port => Ok("nf.data.ciscoServerL4Port"),
            CiscoIPFixField::CiscoConnectionId => Ok("nf.data.ciscoConnectionId"),
            CiscoIPFixField::CiscoAppBusiness => Ok("nf.data.ciscoAppBusiness"),
            CiscoIPFixField::Unknown(field_num) => Err(*field_num),
        }
    }
}

pub fn ipfix_iana_field_map(field: &IANAIPFixField) -> Result<&'static str, u16> {
    match field {
        IANAIPFixField::Reserved => Ok("nf.data.Reserved"),
        IANAIPFixField::OctetDeltaCount => Ok("nf.data.octetDeltaCount"),
        IANAIPFixField::PacketDeltaCount => Ok("nf.data.packetDeltaCount"),
        IANAIPFixField::DeltaFlowCount => Ok("nf.data.deltaFlowCount"),
        IANAIPFixField::ProtocolIdentifier => Ok("nf.data.protocolIdentifier"),
        IANAIPFixField::IpClassOfService => Ok("nf.data.ipClassOfService"),
        IANAIPFixField::TcpControlBits => Ok("nf.data.tcpControlBits"),
        IANAIPFixField::SourceTransportPort => Ok("nf.data.sourceTransportPort"),
        IANAIPFixField::SourceIpv4address => Ok("nf.data.sourceIPv4Address"),
        IANAIPFixField::SourceIpv4prefixLength => Ok("nf.data.sourceIPv4PrefixLength"),
        IANAIPFixField::IngressInterface => Ok("nf.data.ingressInterface"),
        IANAIPFixField::DestinationTransportPort => Ok("nf.data.destinationTransportPort"),
        IANAIPFixField::DestinationIpv4address => Ok("nf.data.destinationIPv4Address"),
        IANAIPFixField::DestinationIpv4prefixLength => Ok("nf.data.destinationIPv4PrefixLength"),
        IANAIPFixField::EgressInterface => Ok("nf.data.egressInterface"),
        IANAIPFixField::IpNextHopIpv4address => Ok("nf.data.ipNextHopIPv4Address"),
        IANAIPFixField::BgpSourceAsNumber => Ok("nf.data.bgpSourceAsNumber"),
        IANAIPFixField::BgpDestinationAsNumber => Ok("nf.data.bgpDestinationAsNumber"),
        IANAIPFixField::BgpNextHopIpv4address => Ok("nf.data.bgpNextHopIPv4Address"),
        IANAIPFixField::PostMcastPacketDeltaCount => Ok("nf.data.postMCastPacketDeltaCount"),
        IANAIPFixField::PostMcastOctetDeltaCount => Ok("nf.data.postMCastOctetDeltaCount"),
        IANAIPFixField::FlowEndSysUpTime => Ok("nf.data.flowEndSysUpTime"),
        IANAIPFixField::FlowStartSysUpTime => Ok("nf.data.flowStartSysUpTime"),
        IANAIPFixField::PostOctetDeltaCount => Ok("nf.data.postOctetDeltaCount"),
        IANAIPFixField::PostPacketDeltaCount => Ok("nf.data.postPacketDeltaCount"),
        IANAIPFixField::MinimumIpTotalLength => Ok("nf.data.minimumIpTotalLength"),
        IANAIPFixField::MaximumIpTotalLength => Ok("nf.data.maximumIpTotalLength"),
        IANAIPFixField::SourceIpv6address => Ok("nf.data.sourceIPv6Address"),
        IANAIPFixField::DestinationIpv6address => Ok("nf.data.destinationIPv6Address"),
        IANAIPFixField::SourceIpv6prefixLength => Ok("nf.data.sourceIPv6PrefixLength"),
        IANAIPFixField::DestinationIpv6prefixLength => Ok("nf.data.destinationIPv6PrefixLength"),
        IANAIPFixField::FlowLabelIpv6 => Ok("nf.data.flowLabelIPv6"),
        IANAIPFixField::IcmpTypeCodeIpv4 => Ok("nf.data.icmpTypeCodeIPv4"),
        IANAIPFixField::IgmpType => Ok("nf.data.igmpType"),
        IANAIPFixField::SamplingInterval => Ok("nf.data.samplingInterval"),
        IANAIPFixField::SamplingAlgorithm => Ok("nf.data.samplingAlgorithm"),
        IANAIPFixField::FlowActiveTimeout => Ok("nf.data.flowActiveTimeout"),
        IANAIPFixField::FlowIdleTimeout => Ok("nf.data.flowIdleTimeout"),
        IANAIPFixField::EngineType => Ok("nf.data.engineType"),
        IANAIPFixField::EngineId => Ok("nf.data.engineId"),
        IANAIPFixField::ExportedOctetTotalCount => Ok("nf.data.exportedOctetTotalCount"),
        IANAIPFixField::ExportedMessageTotalCount => Ok("nf.data.exportedMessageTotalCount"),
        IANAIPFixField::ExportedFlowRecordTotalCount => Ok("nf.data.exportedFlowRecordTotalCount"),
        IANAIPFixField::Ipv4routerSc => Ok("nf.data.ipv4RouterSc"),
        IANAIPFixField::SourceIpv4prefix => Ok("nf.data.sourceIPv4Prefix"),
        IANAIPFixField::DestinationIpv4prefix => Ok("nf.data.destinationIPv4Prefix"),
        IANAIPFixField::MplsTopLabelType => Ok("nf.data.mplsTopLabelType"),
        IANAIPFixField::MplsTopLabelIpv4address => Ok("nf.data.mplsTopLabelIPv4Address"),
        IANAIPFixField::SamplerId => Ok("nf.data.samplerId"),
        IANAIPFixField::SamplerMode => Ok("nf.data.samplerMode"),
        IANAIPFixField::SamplerRandomInterval => Ok("nf.data.samplerRandomInterval"),
        IANAIPFixField::ClassId => Ok("nf.data.classId"),
        IANAIPFixField::MinimumTtl => Ok("nf.data.minimumTTL"),
        IANAIPFixField::MaximumTtl => Ok("nf.data.maximumTTL"),
        IANAIPFixField::FragmentIdentification => Ok("nf.data.fragmentIdentification"),
        IANAIPFixField::PostIpClassOfService => Ok("nf.data.postIpClassOfService"),
        IANAIPFixField::SourceMacaddress => Ok("nf.data.sourceMacAddress"),
        IANAIPFixField::PostDestinationMacaddress => Ok("nf.data.postDestinationMacAddress"),
        IANAIPFixField::VlanId => Ok("nf.data.vlanId"),
        IANAIPFixField::PostVlanId => Ok("nf.data.postVlanId"),
        IANAIPFixField::IpVersion => Ok("nf.data.ipVersion"),
        IANAIPFixField::FlowDirection => Ok("nf.data.flowDirection"),
        IANAIPFixField::IpNextHopIpv6address => Ok("nf.data.ipNextHopIPv6Address"),
        IANAIPFixField::BgpNextHopIpv6address => Ok("nf.data.bgpNextHopIPv6Address"),
        IANAIPFixField::Ipv6extensionHeaders => Ok("nf.data.ipv6ExtensionHeaders"),
        IANAIPFixField::AssignedforNetFlowv9compatibility65 => Err(65),
        IANAIPFixField::AssignedforNetFlowv9compatibility66 => Err(66),
        IANAIPFixField::AssignedforNetFlowv9compatibility67 => Err(67),
        IANAIPFixField::AssignedforNetFlowv9compatibility68 => Err(68),
        IANAIPFixField::AssignedforNetFlowv9compatibility69 => Err(69),
        IANAIPFixField::MplsTopLabelStackSection => Ok("nf.data.mplsTopLabelStackSection"),
        IANAIPFixField::MplsLabelStackSection2 => Ok("nf.data.mplsLabelStackSection2"),
        IANAIPFixField::MplsLabelStackSection3 => Ok("nf.data.mplsLabelStackSection3"),
        IANAIPFixField::MplsLabelStackSection4 => Ok("nf.data.mplsLabelStackSection4"),
        IANAIPFixField::MplsLabelStackSection5 => Ok("nf.data.mplsLabelStackSection5"),
        IANAIPFixField::MplsLabelStackSection6 => Ok("nf.data.mplsLabelStackSection6"),
        IANAIPFixField::MplsLabelStackSection7 => Ok("nf.data.mplsLabelStackSection7"),
        IANAIPFixField::MplsLabelStackSection8 => Ok("nf.data.mplsLabelStackSection8"),
        IANAIPFixField::MplsLabelStackSection9 => Ok("nf.data.mplsLabelStackSection9"),
        IANAIPFixField::MplsLabelStackSection10 => Ok("nf.data.mplsLabelStackSection10"),
        IANAIPFixField::DestinationMacaddress => Ok("nf.data.destinationMacAddress"),
        IANAIPFixField::PostSourceMacaddress => Ok("nf.data.postSourceMacAddress"),
        IANAIPFixField::InterfaceName => Ok("nf.data.interfaceName"),
        IANAIPFixField::InterfaceDescription => Ok("nf.data.interfaceDescription"),
        IANAIPFixField::SamplerName => Ok("nf.data.samplerName"),
        IANAIPFixField::OctetTotalCount => Ok("nf.data.octetTotalCount"),
        IANAIPFixField::PacketTotalCount => Ok("nf.data.packetTotalCount"),
        IANAIPFixField::FlagsAndSamplerId => Ok("nf.data.flagsAndSamplerId"),
        IANAIPFixField::FragmentOffset => Ok("nf.data.fragmentOffset"),
        IANAIPFixField::ForwardingStatus => Ok("nf.data.forwardingStatus"),
        IANAIPFixField::MplsVpnRouteDistinguisher => Ok("nf.data.mplsVpnRouteDistinguisher"),
        IANAIPFixField::MplsTopLabelprefixLength => Ok("nf.data.mplsTopLabelPrefixLength"),
        IANAIPFixField::SrcTrafficIndex => Ok("nf.data.srcTrafficIndex"),
        IANAIPFixField::DstTrafficIndex => Ok("nf.data.dstTrafficIndex"),
        IANAIPFixField::ApplicationDescription => Ok("nf.data.applicationDescription"),
        IANAIPFixField::ApplicationId => Ok("nf.data.applicationId"),
        IANAIPFixField::ApplicationName => Ok("nf.data.applicationName"),
        IANAIPFixField::AssignedforNetFlowv9compatibility97 => Err(97),
        IANAIPFixField::PostIpDiffServCodePoint => Ok("nf.data.postIpDiffServCodePoint"),
        IANAIPFixField::MulticastReplicationFactor => Ok("nf.data.multicastReplicationFactor"),
        IANAIPFixField::ClassName => Ok("nf.data.className"),
        IANAIPFixField::ClassificationEngineId => Ok("nf.data.classificationEngineId"),
        IANAIPFixField::Layer2packetSectionOffset => Ok("nf.data.layer2packetSectionOffset"),
        IANAIPFixField::Layer2packetSectionSize => Ok("nf.data.layer2packetSectionSize"),
        IANAIPFixField::Layer2packetSectionData => Ok("nf.data.layer2packetSectionData"),
        IANAIPFixField::AssignedforNetFlowv9compatibility105 => Err(105),
        IANAIPFixField::AssignedforNetFlowv9compatibility106 => Err(106),
        IANAIPFixField::AssignedforNetFlowv9compatibility107 => Err(107),
        IANAIPFixField::AssignedforNetFlowv9compatibility108 => Err(108),
        IANAIPFixField::AssignedforNetFlowv9compatibility109 => Err(109),
        IANAIPFixField::AssignedforNetFlowv9compatibility110 => Err(110),
        IANAIPFixField::AssignedforNetFlowv9compatibility111 => Err(111),
        IANAIPFixField::AssignedforNetFlowv9compatibility112 => Err(112),
        IANAIPFixField::AssignedforNetFlowv9compatibility113 => Err(113),
        IANAIPFixField::AssignedforNetFlowv9compatibility114 => Err(114),
        IANAIPFixField::AssignedforNetFlowv9compatibility115 => Err(115),
        IANAIPFixField::AssignedforNetFlowv9compatibility116 => Err(116),
        IANAIPFixField::AssignedforNetFlowv9compatibility117 => Err(117),
        IANAIPFixField::AssignedforNetFlowv9compatibility118 => Err(118),
        IANAIPFixField::AssignedforNetFlowv9compatibility119 => Err(119),
        IANAIPFixField::AssignedforNetFlowv9compatibility120 => Err(120),
        IANAIPFixField::AssignedforNetFlowv9compatibility121 => Err(121),
        IANAIPFixField::AssignedforNetFlowv9compatibility122 => Err(122),
        IANAIPFixField::AssignedforNetFlowv9compatibility123 => Err(123),
        IANAIPFixField::AssignedforNetFlowv9compatibility124 => Err(124),
        IANAIPFixField::AssignedforNetFlowv9compatibility125 => Err(125),
        IANAIPFixField::AssignedforNetFlowv9compatibility126 => Err(126),
        IANAIPFixField::AssignedforNetFlowv9compatibility127 => Err(127),
        IANAIPFixField::BgpNextAdjacentAsNumber => Ok("nf.data.bgpNextAdjacentAsNumber"),
        IANAIPFixField::BgpPrevAdjacentAsNumber => Ok("nf.data.bgpPrevAdjacentAsNumber"),
        IANAIPFixField::ExporterIpv4address => Ok("nf.data.exporterIPv4Address"),
        IANAIPFixField::ExporterIpv6address => Ok("nf.data.exporterIPv6Address"),
        IANAIPFixField::DroppedOctetDeltaCount => Ok("nf.data.droppedOctetDeltaCount"),
        IANAIPFixField::DroppedPacketDeltaCount => Ok("nf.data.droppedPacketDeltaCount"),
        IANAIPFixField::DroppedOctetTotalCount => Ok("nf.data.droppedOctetTotalCount"),
        IANAIPFixField::DroppedPacketTotalCount => Ok("nf.data.droppedPacketTotalCount"),
        IANAIPFixField::FlowEndReason => Ok("nf.data.flowEndReason"),
        IANAIPFixField::CommonPropertiesId => Ok("nf.data.commonPropertiesId"),
        IANAIPFixField::ObservationPointId => Ok("nf.data.observationPointId"),
        IANAIPFixField::IcmpTypeCodeIpv6 => Ok("nf.data.icmpTypeCodeIPv6"),
        IANAIPFixField::MplsTopLabelIpv6address => Ok("nf.data.mplsTopLabelIPv6Address"),
        IANAIPFixField::LineCardId => Ok("nf.data.lineCardId"),
        IANAIPFixField::PortId => Ok("nf.data.portId"),
        IANAIPFixField::MeteringProcessId => Ok("nf.data.meteringProcessId"),
        IANAIPFixField::ExportingProcessId => Ok("nf.data.exportingProcessId"),
        IANAIPFixField::TemplateId => Ok("nf.data.templateId"),
        IANAIPFixField::WlanChannelId => Ok("nf.data.wlanChannelId"),
        IANAIPFixField::WlanSsid => Ok("nf.data.wlanSSID"),
        IANAIPFixField::FlowId => Ok("nf.data.flowId"),
        IANAIPFixField::ObservationDomainId => Ok("nf.data.observationDomainId"),
        IANAIPFixField::FlowStartSeconds => Ok("nf.data.flowStartSeconds"),
        IANAIPFixField::FlowEndSeconds => Ok("nf.data.flowEndSeconds"),
        IANAIPFixField::FlowStartMilliseconds => Ok("nf.data.flowStartMilliseconds"),
        IANAIPFixField::FlowEndMilliseconds => Ok("nf.data.flowEndMilliseconds"),
        IANAIPFixField::FlowStartMicroseconds => Ok("nf.data.flowStartMicroseconds"),
        IANAIPFixField::FlowEndMicroseconds => Ok("nf.data.flowEndMicroseconds"),
        IANAIPFixField::FlowStartNanoseconds => Ok("nf.data.flowStartNanoseconds"),
        IANAIPFixField::FlowEndNanoseconds => Ok("nf.data.flowEndNanoseconds"),
        IANAIPFixField::FlowStartDeltaMicroseconds => Ok("nf.data.flowStartDeltaMicroseconds"),
        IANAIPFixField::FlowEndDeltaMicroseconds => Ok("nf.data.flowEndDeltaMicroseconds"),
        IANAIPFixField::SystemInitTimeMilliseconds => Ok("nf.data.systemInitTimeMilliseconds"),
        IANAIPFixField::FlowDurationMilliseconds => Ok("nf.data.flowDurationMilliseconds"),
        IANAIPFixField::FlowDurationMicroseconds => Ok("nf.data.flowDurationMicroseconds"),
        IANAIPFixField::ObservedFlowTotalCount => Ok("nf.data.observedFlowTotalCount"),
        IANAIPFixField::IgnoredPacketTotalCount => Ok("nf.data.ignoredPacketTotalCount"),
        IANAIPFixField::IgnoredOctetTotalCount => Ok("nf.data.ignoredOctetTotalCount"),
        IANAIPFixField::NotSentFlowTotalCount => Ok("nf.data.notSentFlowTotalCount"),
        IANAIPFixField::NotSentPacketTotalCount => Ok("nf.data.notSentPacketTotalCount"),
        IANAIPFixField::NotSentOctetTotalCount => Ok("nf.data.notSentOctetTotalCount"),
        IANAIPFixField::DestinationIpv6prefix => Ok("nf.data.destinationIPv6Prefix"),
        IANAIPFixField::SourceIpv6prefix => Ok("nf.data.sourceIPv6Prefix"),
        IANAIPFixField::PostOctetTotalCount => Ok("nf.data.postOctetTotalCount"),
        IANAIPFixField::PostPacketTotalCount => Ok("nf.data.postPacketTotalCount"),
        IANAIPFixField::FlowKeyIndicator => Ok("nf.data.flowKeyIndicator"),
        IANAIPFixField::PostMcastPacketTotalCount => Ok("nf.data.postMCastPacketTotalCount"),
        IANAIPFixField::PostMcastOctetTotalCount => Ok("nf.data.postMCastOctetTotalCount"),
        IANAIPFixField::IcmpTypeIpv4 => Ok("nf.data.icmpTypeIPv4"),
        IANAIPFixField::IcmpCodeIpv4 => Ok("nf.data.icmpCodeIPv4"),
        IANAIPFixField::IcmpTypeIpv6 => Ok("nf.data.icmpTypeIPv6"),
        IANAIPFixField::IcmpCodeIpv6 => Ok("nf.data.icmpCodeIPv6"),
        IANAIPFixField::UdpSourcePort => Ok("nf.data.udpSourcePort"),
        IANAIPFixField::UdpDestinationPort => Ok("nf.data.udpDestinationPort"),
        IANAIPFixField::TcpSourcePort => Ok("nf.data.tcpSourcePort"),
        IANAIPFixField::TcpDestinationPort => Ok("nf.data.tcpDestinationPort"),
        IANAIPFixField::TcpSequenceNumber => Ok("nf.data.tcpSequenceNumber"),
        IANAIPFixField::TcpAcknowledgementNumber => Ok("nf.data.tcpAcknowledgementNumber"),
        IANAIPFixField::TcpWindowSize => Ok("nf.data.tcpWindowSize"),
        IANAIPFixField::TcpUrgentPointer => Ok("nf.data.tcpUrgentPointer"),
        IANAIPFixField::TcpHeaderLength => Ok("nf.data.tcpHeaderLength"),
        IANAIPFixField::IpHeaderLength => Ok("nf.data.ipHeaderLength"),
        IANAIPFixField::TotalLengthIpv4 => Ok("nf.data.totalLengthIPv4"),
        IANAIPFixField::PayloadLengthIpv6 => Ok("nf.data.payloadLengthIPv6"),
        IANAIPFixField::IpTtl => Ok("nf.data.ipTTL"),
        IANAIPFixField::NextHeaderIpv6 => Ok("nf.data.nextHeaderIPv6"),
        IANAIPFixField::MplsPayloadLength => Ok("nf.data.mplsPayloadLength"),
        IANAIPFixField::IpDiffServCodePoint => Ok("nf.data.ipDiffServCodePoint"),
        IANAIPFixField::IpPrecedence => Ok("nf.data.ipPrecedence"),
        IANAIPFixField::FragmentFlags => Ok("nf.data.fragmentFlags"),
        IANAIPFixField::OctetDeltaSumOfSquares => Ok("nf.data.octetDeltaSumOfSquares"),
        IANAIPFixField::OctetTotalSumOfSquares => Ok("nf.data.octetTotalSumOfSquares"),
        IANAIPFixField::MplsTopLabelTtl => Ok("nf.data.mplsTopLabelTTL"),
        IANAIPFixField::MplsLabelStackLength => Ok("nf.data.mplsLabelStackLength"),
        IANAIPFixField::MplsLabelStackDepth => Ok("nf.data.mplsLabelStackDepth"),
        IANAIPFixField::MplsTopLabelExp => Ok("nf.data.mplsTopLabelExp"),
        IANAIPFixField::IpPayloadLength => Ok("nf.data.ipPayloadLength"),
        IANAIPFixField::UdpMessageLength => Ok("nf.data.udpMessageLength"),
        IANAIPFixField::IsMulticast => Ok("nf.data.isMulticast"),
        IANAIPFixField::Ipv4ihl => Ok("nf.data.ipv4IHL"),
        IANAIPFixField::Ipv4options => Ok("nf.data.ipv4Options"),
        IANAIPFixField::TcpOptions => Ok("nf.data.tcpOptions"),
        IANAIPFixField::PaddingOctets => Ok("nf.data.paddingOctets"),
        IANAIPFixField::CollectorIpv4address => Ok("nf.data.collectorIPv4Address"),
        IANAIPFixField::CollectorIpv6address => Ok("nf.data.collectorIPv6Address"),
        IANAIPFixField::ExportInterface => Ok("nf.data.exportInterface"),
        IANAIPFixField::ExportProtocolVersion => Ok("nf.data.exportProtocolVersion"),
        IANAIPFixField::ExportTransportProtocol => Ok("nf.data.exportTransportProtocol"),
        IANAIPFixField::CollectorTransportPort => Ok("nf.data.collectorTransportPort"),
        IANAIPFixField::ExporterTransportPort => Ok("nf.data.exporterTransportPort"),
        IANAIPFixField::TcpSynTotalCount => Ok("nf.data.tcpSynTotalCount"),
        IANAIPFixField::TcpFinTotalCount => Ok("nf.data.tcpFinTotalCount"),
        IANAIPFixField::TcpRstTotalCount => Ok("nf.data.tcpRstTotalCount"),
        IANAIPFixField::TcpPshTotalCount => Ok("nf.data.tcpPshTotalCount"),
        IANAIPFixField::TcpAckTotalCount => Ok("nf.data.tcpAckTotalCount"),
        IANAIPFixField::TcpUrgTotalCount => Ok("nf.data.tcpUrgTotalCount"),
        IANAIPFixField::IpTotalLength => Ok("nf.data.ipTotalLength"),
        IANAIPFixField::PostNatsourceIpv4address => Ok("nf.data.postNATSourceIPv4Address"),
        IANAIPFixField::PostNatdestinationIpv4address => {
            Ok("nf.data.postNATDestinationIPv4Address")
        }
        IANAIPFixField::PostNaptsourceTransportPort => Ok("nf.data.postNAPTSourceTransportPort"),
        IANAIPFixField::PostNaptdestinationTransportPort => {
            Ok("nf.data.postNAPTDestinationTransportPort")
        }
        IANAIPFixField::NatOriginatingaddressRealm => Ok("nf.data.natOriginatingAddressRealm"),
        IANAIPFixField::NatEvent => Ok("nf.data.natEvent"),
        IANAIPFixField::InitiatorOctets => Ok("nf.data.initiatorOctets"),
        IANAIPFixField::ResponderOctets => Ok("nf.data.responderOctets"),
        IANAIPFixField::FirewallEvent => Ok("nf.data.firewallEvent"),
        IANAIPFixField::IngressVrfid => Ok("nf.data.ingressVRFID"),
        IANAIPFixField::EgressVrfid => Ok("nf.data.egressVRFID"),
        IANAIPFixField::VRFname => Ok("nf.data.VRFname"),
        IANAIPFixField::PostMplsTopLabelExp => Ok("nf.data.postMplsTopLabelExp"),
        IANAIPFixField::TcpWindowScale => Ok("nf.data.tcpWindowScale"),
        IANAIPFixField::BiflowDirection => Ok("nf.data.biflowDirection"),
        IANAIPFixField::EthernetHeaderLength => Ok("nf.data.ethernetHeaderLength"),
        IANAIPFixField::EthernetPayloadLength => Ok("nf.data.ethernetPayloadLength"),
        IANAIPFixField::EthernetTotalLength => Ok("nf.data.ethernetTotalLength"),
        IANAIPFixField::Dot1qVlanId => Ok("nf.data.dot1qVlanId"),
        IANAIPFixField::Dot1qPriority => Ok("nf.data.dot1qPriority"),
        IANAIPFixField::Dot1qCustomerVlanId => Ok("nf.data.dot1qCustomerVlanId"),
        IANAIPFixField::Dot1qCustomerPriority => Ok("nf.data.dot1qCustomerPriority"),
        IANAIPFixField::MetroEvcId => Ok("nf.data.metroEvcId"),
        IANAIPFixField::MetroEvcType => Ok("nf.data.metroEvcType"),
        IANAIPFixField::PseudoWireId => Ok("nf.data.pseudoWireId"),
        IANAIPFixField::PseudoWireType => Ok("nf.data.pseudoWireType"),
        IANAIPFixField::PseudoWireControlWord => Ok("nf.data.pseudoWireControlWord"),
        IANAIPFixField::IngressPhysicalInterface => Ok("nf.data.ingressPhysicalInterface"),
        IANAIPFixField::EgressPhysicalInterface => Ok("nf.data.egressPhysicalInterface"),
        IANAIPFixField::PostDot1qVlanId => Ok("nf.data.postDot1qVlanId"),
        IANAIPFixField::PostDot1qCustomerVlanId => Ok("nf.data.postDot1qCustomerVlanId"),
        IANAIPFixField::EthernetType => Ok("nf.data.ethernetType"),
        IANAIPFixField::PostIpPrecedence => Ok("nf.data.postIpPrecedence"),
        IANAIPFixField::CollectionTimeMilliseconds => Ok("nf.data.collectionTimeMilliseconds"),
        IANAIPFixField::ExportSctpStreamId => Ok("nf.data.exportSctpStreamId"),
        IANAIPFixField::MaxExportSeconds => Ok("nf.data.maxExportSeconds"),
        IANAIPFixField::MaxFlowEndSeconds => Ok("nf.data.maxFlowEndSeconds"),
        IANAIPFixField::MessageMd5checksum => Ok("nf.data.messageMD5Checksum"),
        IANAIPFixField::MessageScope => Ok("nf.data.messageScope"),
        IANAIPFixField::MinExportSeconds => Ok("nf.data.minExportSeconds"),
        IANAIPFixField::MinFlowStartSeconds => Ok("nf.data.minFlowStartSeconds"),
        IANAIPFixField::OpaqueOctets => Ok("nf.data.opaqueOctets"),
        IANAIPFixField::SessionScope => Ok("nf.data.sessionScope"),
        IANAIPFixField::MaxFlowEndMicroseconds => Ok("nf.data.maxFlowEndMicroseconds"),
        IANAIPFixField::MaxFlowEndMilliseconds => Ok("nf.data.maxFlowEndMilliseconds"),
        IANAIPFixField::MaxFlowEndNanoseconds => Ok("nf.data.maxFlowEndNanoseconds"),
        IANAIPFixField::MinFlowStartMicroseconds => Ok("nf.data.minFlowStartMicroseconds"),
        IANAIPFixField::MinFlowStartMilliseconds => Ok("nf.data.minFlowStartMilliseconds"),
        IANAIPFixField::MinFlowStartNanoseconds => Ok("nf.data.minFlowStartNanoseconds"),
        IANAIPFixField::CollectorCertificate => Ok("nf.data.collectorCertificate"),
        IANAIPFixField::ExporterCertificate => Ok("nf.data.exporterCertificate"),
        IANAIPFixField::DataRecordsReliability => Ok("nf.data.dataRecordsReliability"),
        IANAIPFixField::ObservationPointType => Ok("nf.data.observationPointType"),
        IANAIPFixField::NewConnectionDeltaCount => Ok("nf.data.newConnectionDeltaCount"),
        IANAIPFixField::ConnectionSumDurationSeconds => Ok("nf.data.connectionSumDurationSeconds"),
        IANAIPFixField::ConnectionTransactionId => Ok("nf.data.connectionTransactionId"),
        IANAIPFixField::PostNatsourceIpv6address => Ok("nf.data.postNATSourceIPv6Address"),
        IANAIPFixField::PostNatdestinationIpv6address => {
            Ok("nf.data.postNATDestinationIPv6Address")
        }
        IANAIPFixField::NatPoolId => Ok("nf.data.natPoolId"),
        IANAIPFixField::NatPoolName => Ok("nf.data.natPoolName"),
        IANAIPFixField::AnonymizationFlags => Ok("nf.data.anonymizationFlags"),
        IANAIPFixField::AnonymizationTechnique => Ok("nf.data.anonymizationTechnique"),
        IANAIPFixField::InformationElementIndex => Ok("nf.data.informationElementIndex"),
        IANAIPFixField::P2pTechnology => Ok("nf.data.p2pTechnology"),
        IANAIPFixField::TunnelTechnology => Ok("nf.data.tunnelTechnology"),
        IANAIPFixField::EncryptedTechnology => Ok("nf.data.encryptedTechnology"),
        IANAIPFixField::BasicList => Ok("nf.data.basicList"),
        IANAIPFixField::SubTemplateList => Ok("nf.data.subTemplateList"),
        IANAIPFixField::SubTemplateMultiList => Ok("nf.data.subTemplateMultiList"),
        IANAIPFixField::BgpValidityState => Ok("nf.data.bgpValidityState"),
        IANAIPFixField::IpSecSPI => Ok("nf.data.IPSecSPI"),
        IANAIPFixField::GreKey => Ok("nf.data.greKey"),
        IANAIPFixField::NatType => Ok("nf.data.natType"),
        IANAIPFixField::InitiatorPackets => Ok("nf.data.initiatorPackets"),
        IANAIPFixField::ResponderPackets => Ok("nf.data.responderPackets"),
        IANAIPFixField::ObservationDomainName => Ok("nf.data.observationDomainName"),
        IANAIPFixField::SelectionSequenceId => Ok("nf.data.selectionSequenceId"),
        IANAIPFixField::SelectorId => Ok("nf.data.selectorId"),
        IANAIPFixField::InformationElementId => Ok("nf.data.informationElementId"),
        IANAIPFixField::SelectorAlgorithm => Ok("nf.data.selectorAlgorithm"),
        IANAIPFixField::SamplingPacketInterval => Ok("nf.data.samplingPacketInterval"),
        IANAIPFixField::SamplingPacketSpace => Ok("nf.data.samplingPacketSpace"),
        IANAIPFixField::SamplingTimeInterval => Ok("nf.data.samplingTimeInterval"),
        IANAIPFixField::SamplingTimeSpace => Ok("nf.data.samplingTimeSpace"),
        IANAIPFixField::SamplingSize => Ok("nf.data.samplingSize"),
        IANAIPFixField::SamplingPopulation => Ok("nf.data.samplingPopulation"),
        IANAIPFixField::SamplingProbability => Ok("nf.data.samplingProbability"),
        IANAIPFixField::DataLinkFrameSize => Ok("nf.data.dataLinkFrameSize"),
        IANAIPFixField::IpHeaderPacketSection => Ok("nf.data.ipHeaderPacketSection"),
        IANAIPFixField::IpPayloadPacketSection => Ok("nf.data.ipPayloadPacketSection"),
        IANAIPFixField::DataLinkFrameSection => Ok("nf.data.dataLinkFrameSection"),
        IANAIPFixField::MplsLabelStackSection => Ok("nf.data.mplsLabelStackSection"),
        IANAIPFixField::MplsPayloadPacketSection => Ok("nf.data.mplsPayloadPacketSection"),
        IANAIPFixField::SelectorIdTotalPktsObserved => Ok("nf.data.selectorIdTotalPktsObserved"),
        IANAIPFixField::SelectorIdTotalPktsSelected => Ok("nf.data.selectorIdTotalPktsSelected"),
        IANAIPFixField::AbsoluteError => Ok("nf.data.absoluteError"),
        IANAIPFixField::RelativeError => Ok("nf.data.relativeError"),
        IANAIPFixField::ObservationTimeSeconds => Ok("nf.data.observationTimeSeconds"),
        IANAIPFixField::ObservationTimeMilliseconds => Ok("nf.data.observationTimeMilliseconds"),
        IANAIPFixField::ObservationTimeMicroseconds => Ok("nf.data.observationTimeMicroseconds"),
        IANAIPFixField::ObservationTimeNanoseconds => Ok("nf.data.observationTimeNanoseconds"),
        IANAIPFixField::DigestHashValue => Ok("nf.data.digestHashValue"),
        IANAIPFixField::HashIppayloadOffset => Ok("nf.data.hashIPPayloadOffset"),
        IANAIPFixField::HashIppayloadSize => Ok("nf.data.hashIPPayloadSize"),
        IANAIPFixField::HashOutputRangeMin => Ok("nf.data.hashOutputRangeMin"),
        IANAIPFixField::HashOutputRangeMax => Ok("nf.data.hashOutputRangeMax"),
        IANAIPFixField::HashSelectedRangeMin => Ok("nf.data.hashSelectedRangeMin"),
        IANAIPFixField::HashSelectedRangeMax => Ok("nf.data.hashSelectedRangeMax"),
        IANAIPFixField::HashDigestOutput => Ok("nf.data.hashDigestOutput"),
        IANAIPFixField::HashInitialiserValue => Ok("nf.data.hashInitialiserValue"),
        IANAIPFixField::SelectorName => Ok("nf.data.selectorName"),
        IANAIPFixField::UpperCilimit => Ok("nf.data.upperCILimit"),
        IANAIPFixField::LowerCilimit => Ok("nf.data.lowerCILimit"),
        IANAIPFixField::ConfidenceLevel => Ok("nf.data.confidenceLevel"),
        IANAIPFixField::InformationElementDataType => Ok("nf.data.informationElementDataType"),
        IANAIPFixField::InformationElementDescription => {
            Ok("nf.data.informationElementDescription")
        }
        IANAIPFixField::InformationElementName => Ok("nf.data.informationElementName"),
        IANAIPFixField::InformationElementRangeBegin => Ok("nf.data.informationElementRangeBegin"),
        IANAIPFixField::InformationElementRangeEnd => Ok("nf.data.informationElementRangeEnd"),
        IANAIPFixField::InformationElementSemantics => Ok("nf.data.informationElementSemantics"),
        IANAIPFixField::InformationElementUnits => Ok("nf.data.informationElementUnits"),
        IANAIPFixField::PrivateEnterpriseNumber => Ok("nf.data.privateEnterpriseNumber"),
        IANAIPFixField::VirtualStationInterfaceId => Ok("nf.data.virtualStationInterfaceId"),
        IANAIPFixField::VirtualStationInterfaceName => Ok("nf.data.virtualStationInterfaceName"),
        IANAIPFixField::VirtualStationUuid => Ok("nf.data.virtualStationUUID"),
        IANAIPFixField::VirtualStationName => Ok("nf.data.virtualStationName"),
        IANAIPFixField::Layer2segmentId => Ok("nf.data.layer2SegmentId"),
        IANAIPFixField::Layer2octetDeltaCount => Ok("nf.data.layer2OctetDeltaCount"),
        IANAIPFixField::Layer2octetTotalCount => Ok("nf.data.layer2OctetTotalCount"),
        IANAIPFixField::IngressUnicastPacketTotalCount => {
            Ok("nf.data.ingressUnicastPacketTotalCount")
        }
        IANAIPFixField::IngressMulticastPacketTotalCount => {
            Ok("nf.data.ingressMulticastPacketTotalCount")
        }
        IANAIPFixField::IngressBroadcastPacketTotalCount => {
            Ok("nf.data.ingressBroadcastPacketTotalCount")
        }
        IANAIPFixField::EgressUnicastPacketTotalCount => {
            Ok("nf.data.egressUnicastPacketTotalCount")
        }
        IANAIPFixField::EgressBroadcastPacketTotalCount => {
            Ok("nf.data.egressBroadcastPacketTotalCount")
        }
        IANAIPFixField::MonitoringIntervalStartMilliSeconds => {
            Ok("nf.data.monitoringIntervalStartMilliSeconds")
        }
        IANAIPFixField::MonitoringIntervalEndMilliSeconds => {
            Ok("nf.data.monitoringIntervalEndMilliSeconds")
        }
        IANAIPFixField::PortRangeStart => Ok("nf.data.portRangeStart"),
        IANAIPFixField::PortRangeEnd => Ok("nf.data.portRangeEnd"),
        IANAIPFixField::PortRangeStepSize => Ok("nf.data.portRangeStepSize"),
        IANAIPFixField::PortRangeNumPorts => Ok("nf.data.portRangeNumPorts"),
        IANAIPFixField::StaMacaddress => Ok("nf.data.staMacAddress"),
        IANAIPFixField::StaIpv4address => Ok("nf.data.staIPv4Address"),
        IANAIPFixField::WtpMacaddress => Ok("nf.data.wtpMacAddress"),
        IANAIPFixField::IngressInterfaceType => Ok("nf.data.ingressInterfaceType"),
        IANAIPFixField::EgressInterfaceType => Ok("nf.data.egressInterfaceType"),
        IANAIPFixField::RtpSequenceNumber => Ok("nf.data.rtpSequenceNumber"),
        IANAIPFixField::UserName => Ok("nf.data.userName"),
        IANAIPFixField::ApplicationCategoryName => Ok("nf.data.applicationCategoryName"),
        IANAIPFixField::ApplicationSubCategoryName => Ok("nf.data.applicationSubCategoryName"),
        IANAIPFixField::ApplicationGroupName => Ok("nf.data.applicationGroupName"),
        IANAIPFixField::OriginalFlowsPresent => Ok("nf.data.originalFlowsPresent"),
        IANAIPFixField::OriginalFlowsInitiated => Ok("nf.data.originalFlowsInitiated"),
        IANAIPFixField::OriginalFlowsCompleted => Ok("nf.data.originalFlowsCompleted"),
        IANAIPFixField::DistinctCountOfSourceIpaddress => {
            Ok("nf.data.distinctCountOfSourceIPAddress")
        }
        IANAIPFixField::DistinctCountOfDestinationIpaddress => {
            Ok("nf.data.distinctCountOfDestinationIPAddress")
        }
        IANAIPFixField::DistinctCountOfSourceIpv4address => {
            Ok("nf.data.distinctCountOfSourceIPv4Address")
        }
        IANAIPFixField::DistinctCountOfDestinationIpv4address => {
            Ok("nf.data.distinctCountOfDestinationIPv4Address")
        }
        IANAIPFixField::DistinctCountOfSourceIpv6address => {
            Ok("nf.data.distinctCountOfSourceIPv6Address")
        }
        IANAIPFixField::DistinctCountOfDestinationIpv6address => {
            Ok("nf.data.distinctCountOfDestinationIPv6Address")
        }
        IANAIPFixField::ValueDistributionMethod => Ok("nf.data.valueDistributionMethod"),
        IANAIPFixField::Rfc3550jitterMilliseconds => Ok("nf.data.rfc3550JitterMilliseconds"),
        IANAIPFixField::Rfc3550jitterMicroseconds => Ok("nf.data.rfc3550JitterMicroseconds"),
        IANAIPFixField::Rfc3550jitterNanoseconds => Ok("nf.data.rfc3550JitterNanoseconds"),
        IANAIPFixField::Dot1qDei => Ok("nf.data.dot1qDEI"),
        IANAIPFixField::Dot1qCustomerDei => Ok("nf.data.dot1qCustomerDEI"),
        IANAIPFixField::FlowSelectorAlgorithm => Ok("nf.data.flowSelectorAlgorithm"),
        IANAIPFixField::FlowSelectedOctetDeltaCount => Ok("nf.data.flowSelectedOctetDeltaCount"),
        IANAIPFixField::FlowSelectedPacketDeltaCount => Ok("nf.data.flowSelectedPacketDeltaCount"),
        IANAIPFixField::FlowSelectedFlowDeltaCount => Ok("nf.data.flowSelectedFlowDeltaCount"),
        IANAIPFixField::SelectorIdtotalFlowsObserved => Ok("nf.data.selectorIDTotalFlowsObserved"),
        IANAIPFixField::SelectorIdtotalFlowsSelected => Ok("nf.data.selectorIDTotalFlowsSelected"),
        IANAIPFixField::SamplingFlowInterval => Ok("nf.data.samplingFlowInterval"),
        IANAIPFixField::SamplingFlowSpacing => Ok("nf.data.samplingFlowSpacing"),
        IANAIPFixField::FlowSamplingTimeInterval => Ok("nf.data.flowSamplingTimeInterval"),
        IANAIPFixField::FlowSamplingTimeSpacing => Ok("nf.data.flowSamplingTimeSpacing"),
        IANAIPFixField::HashFlowDomain => Ok("nf.data.hashFlowDomain"),
        IANAIPFixField::TransportOctetDeltaCount => Ok("nf.data.transportOctetDeltaCount"),
        IANAIPFixField::TransportPacketDeltaCount => Ok("nf.data.transportPacketDeltaCount"),
        IANAIPFixField::OriginalExporterIpv4address => Ok("nf.data.originalExporterIPv4Address"),
        IANAIPFixField::OriginalExporterIpv6address => Ok("nf.data.originalExporterIPv6Address"),
        IANAIPFixField::OriginalObservationDomainId => Ok("nf.data.originalObservationDomainId"),
        IANAIPFixField::IntermediateProcessId => Ok("nf.data.intermediateProcessId"),
        IANAIPFixField::IgnoredDataRecordTotalCount => Ok("nf.data.ignoredDataRecordTotalCount"),
        IANAIPFixField::DataLinkFrameType => Ok("nf.data.dataLinkFrameType"),
        IANAIPFixField::SectionOffset => Ok("nf.data.sectionOffset"),
        IANAIPFixField::SectionExportedOctets => Ok("nf.data.sectionExportedOctets"),
        IANAIPFixField::Dot1qServiceInstanceTag => Ok("nf.data.dot1qServiceInstanceTag"),
        IANAIPFixField::Dot1qServiceInstanceId => Ok("nf.data.dot1qServiceInstanceId"),
        IANAIPFixField::Dot1qServiceInstancePriority => Ok("nf.data.dot1qServiceInstancePriority"),
        IANAIPFixField::Dot1qCustomerSourceMacaddress => {
            Ok("nf.data.dot1qCustomerSourceMacAddress")
        }
        IANAIPFixField::Dot1qCustomerDestinationMacaddress => {
            Ok("nf.data.dot1qCustomerDestinationMacAddress")
        }
        IANAIPFixField::PostLayer2octetDeltaCount => Ok("nf.data.postLayer2OctetDeltaCount"),
        IANAIPFixField::PostMcastLayer2octetDeltaCount => {
            Ok("nf.data.postMCastLayer2OctetDeltaCount")
        }
        IANAIPFixField::PostLayer2octetTotalCount => Ok("nf.data.postLayer2OctetTotalCount"),
        IANAIPFixField::PostMcastLayer2octetTotalCount => {
            Ok("nf.data.postMCastLayer2OctetTotalCount")
        }
        IANAIPFixField::MinimumLayer2totalLength => Ok("nf.data.minimumLayer2TotalLength"),
        IANAIPFixField::MaximumLayer2totalLength => Ok("nf.data.maximumLayer2TotalLength"),
        IANAIPFixField::DroppedLayer2octetDeltaCount => Ok("nf.data.droppedLayer2OctetDeltaCount"),
        IANAIPFixField::DroppedLayer2octetTotalCount => Ok("nf.data.droppedLayer2OctetTotalCount"),
        IANAIPFixField::IgnoredLayer2octetTotalCount => Ok("nf.data.ignoredLayer2OctetTotalCount"),
        IANAIPFixField::NotSentLayer2octetTotalCount => Ok("nf.data.notSentLayer2OctetTotalCount"),
        IANAIPFixField::Layer2octetDeltaSumOfSquares => Ok("nf.data.layer2OctetDeltaSumOfSquares"),
        IANAIPFixField::Layer2octetTotalSumOfSquares => Ok("nf.data.layer2OctetTotalSumOfSquares"),
        IANAIPFixField::Layer2frameDeltaCount => Ok("nf.data.layer2FrameDeltaCount"),
        IANAIPFixField::Layer2frameTotalCount => Ok("nf.data.layer2FrameTotalCount"),
        IANAIPFixField::PseudoWireDestinationIpv4address => {
            Ok("nf.data.pseudoWireDestinationIPv4Address")
        }
        IANAIPFixField::IgnoredLayer2frameTotalCount => Ok("nf.data.ignoredLayer2FrameTotalCount"),
        IANAIPFixField::MibObjectValueInteger => Ok("nf.data.mibObjectValueInteger"),
        IANAIPFixField::MibObjectValueOctetString => Ok("nf.data.mibObjectValueOctetString"),
        IANAIPFixField::MibObjectValueOid => Ok("nf.data.mibObjectValueOID"),
        IANAIPFixField::MibObjectValueBits => Ok("nf.data.mibObjectValueBits"),
        IANAIPFixField::MibObjectValueIpaddress => Ok("nf.data.mibObjectValueIPAddress"),
        IANAIPFixField::MibObjectValueCounter => Ok("nf.data.mibObjectValueCounter"),
        IANAIPFixField::MibObjectValueGauge => Ok("nf.data.mibObjectValueGauge"),
        IANAIPFixField::MibObjectValueTimeTicks => Ok("nf.data.mibObjectValueTimeTicks"),
        IANAIPFixField::MibObjectValueUnsigned => Ok("nf.data.mibObjectValueUnsigned"),
        IANAIPFixField::MibObjectValueTable => Ok("nf.data.mibObjectValueTable"),
        IANAIPFixField::MibObjectValueRow => Ok("nf.data.mibObjectValueRow"),
        IANAIPFixField::MibObjectIdentifier => Ok("nf.data.mibObjectIdentifier"),
        IANAIPFixField::MibSubIdentifier => Ok("nf.data.mibSubIdentifier"),
        IANAIPFixField::MibIndexIndicator => Ok("nf.data.mibIndexIndicator"),
        IANAIPFixField::MibCaptureTimeSemantics => Ok("nf.data.mibCaptureTimeSemantics"),
        IANAIPFixField::MibContextEngineId => Ok("nf.data.mibContextEngineID"),
        IANAIPFixField::MibContextName => Ok("nf.data.mibContextName"),
        IANAIPFixField::MibObjectName => Ok("nf.data.mibObjectName"),
        IANAIPFixField::MibObjectDescription => Ok("nf.data.mibObjectDescription"),
        IANAIPFixField::MibObjectSyntax => Ok("nf.data.mibObjectSyntax"),
        IANAIPFixField::MibModuleName => Ok("nf.data.mibModuleName"),
        IANAIPFixField::MobileImsi => Ok("nf.data.mobileIMSI"),
        IANAIPFixField::MobileMsisdn => Ok("nf.data.mobileMSISDN"),
        IANAIPFixField::HttpStatusCode => Ok("nf.data.httpStatusCode"),
        IANAIPFixField::SourceTransportPortsLimit => Ok("nf.data.sourceTransportPortsLimit"),
        IANAIPFixField::HttpRequestMethod => Ok("nf.data.httpRequestMethod"),
        IANAIPFixField::HttpRequestHost => Ok("nf.data.httpRequestHost"),
        IANAIPFixField::HttpRequestTarget => Ok("nf.data.httpRequestTarget"),
        IANAIPFixField::HttpMessageVersion => Ok("nf.data.httpMessageVersion"),
        IANAIPFixField::NatInstanceId => Ok("nf.data.natInstanceID"),
        IANAIPFixField::InternaladdressRealm => Ok("nf.data.internalAddressRealm"),
        IANAIPFixField::ExternaladdressRealm => Ok("nf.data.externalAddressRealm"),
        IANAIPFixField::NatQuotaExceededEvent => Ok("nf.data.natQuotaExceededEvent"),
        IANAIPFixField::NatThresholdEvent => Ok("nf.data.natThresholdEvent"),
        IANAIPFixField::HttpUserAgent => Ok("nf.data.httpUserAgent"),
        IANAIPFixField::HttpContentType => Ok("nf.data.httpContentType"),
        IANAIPFixField::HttpReasonPhrase => Ok("nf.data.httpReasonPhrase"),
        IANAIPFixField::MaxSessionEntries => Ok("nf.data.maxSessionEntries"),
        IANAIPFixField::MaxBibentries => Ok("nf.data.maxBIBEntries"),
        IANAIPFixField::MaxEntriesPerUser => Ok("nf.data.maxEntriesPerUser"),
        IANAIPFixField::MaxSubscribers => Ok("nf.data.maxSubscribers"),
        IANAIPFixField::MaxFragmentsPendingReassembly => {
            Ok("nf.data.maxFragmentsPendingReassembly")
        }
        IANAIPFixField::AddressPoolHighThreshold => Ok("nf.data.addressPoolHighThreshold"),
        IANAIPFixField::AddressPoolLowThreshold => Ok("nf.data.addressPoolLowThreshold"),
        IANAIPFixField::AddressPortMappingHighThreshold => {
            Ok("nf.data.addressPortMappingHighThreshold")
        }
        IANAIPFixField::AddressPortMappingLowThreshold => {
            Ok("nf.data.addressPortMappingLowThreshold")
        }
        IANAIPFixField::AddressPortMappingPerUserHighThreshold => {
            Ok("nf.data.addressPortMappingPerUserHighThreshold")
        }
        IANAIPFixField::GlobaladdressMappingHighThreshold => {
            Ok("nf.data.globalAddressMappingHighThreshold")
        }
        IANAIPFixField::VpnIdentifier => Ok("nf.data.vpnIdentifier"),
        IANAIPFixField::BgpCommunity => Ok("nf.data.bgpCommunity"),
        IANAIPFixField::BgpSourceCommunityList => Ok("nf.data.bgpSourceCommunityList"),
        IANAIPFixField::BgpDestinationCommunityList => Ok("nf.data.bgpDestinationCommunityList"),
        IANAIPFixField::BgpExtendedCommunity => Ok("nf.data.bgpExtendedCommunity"),
        IANAIPFixField::BgpSourceExtendedCommunityList => {
            Ok("nf.data.bgpSourceExtendedCommunityList")
        }
        IANAIPFixField::BgpDestinationExtendedCommunityList => {
            Ok("nf.data.bgpDestinationExtendedCommunityList")
        }
        IANAIPFixField::BgpLargeCommunity => Ok("nf.data.bgpLargeCommunity"),
        IANAIPFixField::BgpSourceLargeCommunityList => Ok("nf.data.bgpSourceLargeCommunityList"),
        IANAIPFixField::BgpDestinationLargeCommunityList => {
            Ok("nf.data.bgpDestinationLargeCommunityList")
        }
        IANAIPFixField::SrhFlagsIpv6 => Ok("nf.data.srhFlagsIPv6"),
        IANAIPFixField::SrhTagIpv6 => Ok("nf.data.srhTagIPv6"),
        IANAIPFixField::SrhSegmentIpv6 => Ok("nf.data.srhSegmentIPv6"),
        IANAIPFixField::SrhActiveSegmentIpv6 => Ok("nf.data.srhActiveSegmentIPv6"),
        IANAIPFixField::SrhSegmentIpv6basicList => Ok("nf.data.srhSegmentIPv6BasicList"),
        IANAIPFixField::SrhSegmentIpv6listSection => Ok("nf.data.srhSegmentIPv6ListSection"),
        IANAIPFixField::SrhSegmentsIpv6left => Ok("nf.data.srhSegmentsIPv6Left"),
        IANAIPFixField::SrhIpv6section => Ok("nf.data.srhIPv6Section"),
        IANAIPFixField::SrhIpv6activeSegmentType => Ok("nf.data.srhIPv6ActiveSegmentType"),
        IANAIPFixField::SrhSegmentIpv6locatorLength => Ok("nf.data.srhSegmentIPv6LocatorLength"),
        IANAIPFixField::SrhSegmentIpv6endpointBehavior => {
            Ok("nf.data.srhSegmentIPv6EndpointBehavior")
        }
        IANAIPFixField::Unknown(field_num) => match field_num {
            503 => Ok("nf.data.transportChecksum"),
            504 => Ok("nf.data.icmpHeaderPacketSection"),
            505 => Ok("nf.data.gtpuFlags"),
            506 => Ok("nf.data.gtpuMsgType"),
            507 => Ok("nf.data.gtpuTEid"),
            508 => Ok("nf.data.gtpuSequenceNum"),
            509 => Ok("nf.data.gtpuQFI"),
            510 => Ok("nf.data.gtpuPduType"),
            511 => Ok("nf.data.bgpSourceAsPathList"),
            512 => Ok("nf.data.bgpDestinationAsPathList"),
            513 => Ok("nf.data.ipv6ExtensionHeaderType"),
            514 => Ok("nf.data.ipv6ExtensionHeaderCount"),
            515 => Ok("nf.data.ipv6ExtensionHeadersFull"),
            516 => Ok("nf.data.ipv6ExtensionHeaderTypeCountList"),
            517 => Ok("nf.data.ipv6ExtensionHeadersLimit"),
            518 => Ok("nf.data.ipv6ExtensionHeadersChainLength"),
            519 => Ok("nf.data.ipv6ExtensionHeaderChainLengthList"),
            520 => Ok("nf.data.tcpOptionsFull"),
            521 => Ok("nf.data.tcpSharedOptionExID16"),
            522 => Ok("nf.data.tcpSharedOptionExID32"),
            523 => Ok("nf.data.tcpSharedOptionExID16List"),
            524 => Ok("nf.data.tcpSharedOptionExID32List"),
            525 => Ok("nf.data.udpSafeOptions"),
            526 => Ok("nf.data.udpUnsafeOptions"),
            527 => Ok("nf.data.udpExID"),
            528 => Ok("nf.data.udpSafeExIDList"),
            529 => Ok("nf.data.udpUnsafeExIDList"),
            field_num => Err(*field_num),
        },
    }
}

pub fn ipfix_netscaler_field_map(field: &NetscalerIPFixField) -> Result<&'static str, u16> {
    match field {
        NetscalerIPFixField::NetscalerRoundTripTime => Ok("nf.data.netscalerRoundTripTime"),
        NetscalerIPFixField::NetscalerTransactionId => Ok("nf.data.netscalerTransactionId"),
        NetscalerIPFixField::NetscalerHttpReqUrl => Ok("nf.data.netscalerHttpReqUrl"),
        NetscalerIPFixField::NetscalerHttpReqCookie => Ok("nf.data.netscalerHttpReqCookie"),
        NetscalerIPFixField::NetscalerFlowFlags => Ok("nf.data.netscalerFlowFlags"),
        NetscalerIPFixField::NetscalerConnectionId => Ok("nf.data.netscalerConnectionId"),
        NetscalerIPFixField::NetscalerSyslogPriority => Ok("nf.data.netscalerSyslogPriority"),
        NetscalerIPFixField::NetscalerSyslogMessage => Ok("nf.data.netscalerSyslogMessage"),
        NetscalerIPFixField::NetscalerSyslogTimestamp => Ok("nf.data.netscalerSyslogTimestamp"),
        NetscalerIPFixField::NetscalerHttpReqReferer => Ok("nf.data.netscalerHttpReqReferer"),
        NetscalerIPFixField::NetscalerHttpReqMethod => Ok("nf.data.netscalerHttpReqMethod"),
        NetscalerIPFixField::NetscalerHttpReqHost => Ok("nf.data.netscalerHttpReqHost"),
        NetscalerIPFixField::NetscalerHttpReqUserAgent => Ok("nf.data.netscalerHttpReqUserAgent"),
        NetscalerIPFixField::NetscalerHttpRspStatus => Ok("nf.data.netscalerHttpRspStatus"),
        NetscalerIPFixField::NetscalerHttpRspLen => Ok("nf.data.netscalerHttpRspLen"),
        NetscalerIPFixField::NetscalerServerTTFB => Ok("nf.data.netscalerServerTTFB"),
        NetscalerIPFixField::NetscalerServerTTLB => Ok("nf.data.netscalerServerTTLB"),
        NetscalerIPFixField::NetscalerAppNameIncarnationNumber => {
            Ok("nf.data.netscalerAppNameIncarnationNumber")
        }
        NetscalerIPFixField::NetscalerAppNameAppId => Ok("nf.data.netscalerAppNameAppId"),
        NetscalerIPFixField::NetscalerAppName => Ok("nf.data.netscalerAppName"),
        NetscalerIPFixField::NetscalerHttpReqRcvFB => Ok("nf.data.netscalerHttpReqRcvFB"),
        NetscalerIPFixField::NetscalerHttpReqForwFB => Ok("nf.data.netscalerHttpReqForwFB"),
        NetscalerIPFixField::NetscalerHttpResRcvFB => Ok("nf.data.netscalerHttpResRcvFB"),
        NetscalerIPFixField::NetscalerHttpResForwFB => Ok("nf.data.netscalerHttpResForwFB"),
        NetscalerIPFixField::NetscalerHttpReqRcvLB => Ok("nf.data.netscalerHttpReqRcvLB"),
        NetscalerIPFixField::NetscalerHttpReqForwLB => Ok("nf.data.netscalerHttpReqForwLB"),
        NetscalerIPFixField::NetscalerMainPageId => Ok("nf.data.netscalerMainPageId"),
        NetscalerIPFixField::NetscalerMainPageCoreId => Ok("nf.data.netscalerMainPageCoreId"),
        NetscalerIPFixField::NetscalerHttpClientInteractionStartTime => {
            Ok("nf.data.netscalerHttpClientInteractionStartTime")
        }
        NetscalerIPFixField::NetscalerHttpClientRenderEndTime => {
            Ok("nf.data.netscalerHttpClientRenderEndTime")
        }
        NetscalerIPFixField::NetscalerHttpClientRenderStartTime => {
            Ok("nf.data.netscalerHttpClientRenderStartTime")
        }
        NetscalerIPFixField::NetscalerAppTemplateName => Ok("nf.data.netscalerAppTemplateName"),
        NetscalerIPFixField::NetscalerHttpClientInteractionEndTime => {
            Ok("nf.data.netscalerHttpClientInteractionEndTime")
        }
        NetscalerIPFixField::NetscalerHttpResRcvLB => Ok("nf.data.netscalerHttpResRcvLB"),
        NetscalerIPFixField::NetscalerHttpResForwLB => Ok("nf.data.netscalerHttpResForwLB"),
        NetscalerIPFixField::NetscalerAppUnitNameAppId => Ok("nf.data.netscalerAppUnitNameAppId"),
        NetscalerIPFixField::NetscalerDbLoginFlags => Ok("nf.data.netscalerDbLoginFlags"),
        NetscalerIPFixField::NetscalerDbReqType => Ok("nf.data.netscalerDbReqType"),
        NetscalerIPFixField::NetscalerDbProtocolName => Ok("nf.data.netscalerDbProtocolName"),
        NetscalerIPFixField::NetscalerDbUserName => Ok("nf.data.netscalerDbUserName"),
        NetscalerIPFixField::NetscalerDbDatabaseName => Ok("nf.data.netscalerDbDatabaseName"),
        NetscalerIPFixField::NetscalerDbCltHostName => Ok("nf.data.netscalerDbCltHostName"),
        NetscalerIPFixField::NetscalerDbReqString => Ok("nf.data.netscalerDbReqString"),
        NetscalerIPFixField::NetscalerDbRespStatusString => {
            Ok("nf.data.netscalerDbRespStatusString")
        }
        NetscalerIPFixField::NetscalerDbRespStatus => Ok("nf.data.netscalerDbRespStatus"),
        NetscalerIPFixField::NetscalerDbRespLength => Ok("nf.data.netscalerDbRespLength"),
        NetscalerIPFixField::NetscalerClientRTT => Ok("nf.data.netscalerClientRTT"),
        NetscalerIPFixField::NetscalerHttpContentType => Ok("nf.data.netscalerHttpContentType"),
        NetscalerIPFixField::NetscalerHttpReqAuthorization => {
            Ok("nf.data.netscalerHttpReqAuthorization")
        }
        NetscalerIPFixField::NetscalerHttpReqVia => Ok("nf.data.netscalerHttpReqVia"),
        NetscalerIPFixField::NetscalerHttpResLocation => Ok("nf.data.netscalerHttpResLocation"),
        NetscalerIPFixField::NetscalerHttpResSetCookie => Ok("nf.data.netscalerHttpResSetCookie"),
        NetscalerIPFixField::NetscalerHttpResSetCookie2 => Ok("nf.data.netscalerHttpResSetCookie2"),
        NetscalerIPFixField::NetscalerHttpReqXForwardedFor => {
            Ok("nf.data.netscalerHttpReqXForwardedFor")
        }
        NetscalerIPFixField::NetscalerConnectionChainID => Ok("nf.data.netscalerConnectionChainID"),
        NetscalerIPFixField::NetscalerConnectionChainHopCount => {
            Ok("nf.data.netscalerConnectionChainHopCount")
        }
        NetscalerIPFixField::NetscalerICASessionGuid => Ok("nf.data.netscalerICASessionGuid"),
        NetscalerIPFixField::NetscaleIcaClientVersion => Ok("nf.data.netscaleIcaClientVersion"),
        NetscalerIPFixField::NetscalerIcaClientType => Ok("nf.data.netscalerIcaClientType"),
        NetscalerIPFixField::NetscalerIcaClientIP => Ok("nf.data.netscalerIcaClientIP"),
        NetscalerIPFixField::NetscalerIcaClientHostName => Ok("nf.data.netscalerIcaClientHostName"),
        NetscalerIPFixField::NetscalerAaaUsername => Ok("nf.data.netscalerAaaUsername"),
        NetscalerIPFixField::NetscalerIcaDomainName => Ok("nf.data.netscalerIcaDomainName"),
        NetscalerIPFixField::NetscalerIcaClientLauncher => Ok("nf.data.netscalerIcaClientLauncher"),
        NetscalerIPFixField::NetscalerIcaSessionSetupTime => {
            Ok("nf.data.netscalerIcaSessionSetupTime")
        }
        NetscalerIPFixField::NetscalerIcaServerName => Ok("nf.data.netscalerIcaServerName"),
        NetscalerIPFixField::NetscalerIcaSessionReconnects => {
            Ok("nf.data.netscalerIcaSessionReconnects")
        }
        NetscalerIPFixField::NetscalerIcaRTT => Ok("nf.data.netscalerIcaRTT"),
        NetscalerIPFixField::NetscalerIcaClientsideRXBytes => {
            Ok("nf.data.netscalerIcaClientsideRXBytes")
        }
        NetscalerIPFixField::NetscalerIcaClientsideTXBytes => {
            Ok("nf.data.netscalerIcaClientsideTXBytes")
        }
        NetscalerIPFixField::NetscalerIcaClientsidePacketsRetransmit => {
            Ok("nf.data.netscalerIcaClientsidePacketsRetransmit")
        }
        NetscalerIPFixField::NetscalerIcaServersidePacketsRetransmit => {
            Ok("nf.data.netscalerIcaServersidePacketsRetransmit")
        }
        NetscalerIPFixField::NetscalerIcaClientsideRTT => Ok("nf.data.netscalerIcaClientsideRTT"),
        NetscalerIPFixField::NetscalerIcaServersideRTT => Ok("nf.data.netscalerIcaServersideRTT"),
        NetscalerIPFixField::NetscalerIcaSessionUpdateBeginSec => {
            Ok("nf.data.netscalerIcaSessionUpdateBeginSec")
        }
        NetscalerIPFixField::NetscalerIcaSessionUpdateEndSec => {
            Ok("nf.data.netscalerIcaSessionUpdateEndSec")
        }
        NetscalerIPFixField::NetscalerIcaChannelId1 => Ok("nf.data.netscalerIcaChannelId1"),
        NetscalerIPFixField::NetscalerIcaChannelId1Bytes => {
            Ok("nf.data.netscalerIcaChannelId1Bytes")
        }
        NetscalerIPFixField::NetscalerIcaChannelId2 => Ok("nf.data.netscalerIcaChannelId2"),
        NetscalerIPFixField::NetscalerIcaChannelId2Bytes => {
            Ok("nf.data.netscalerIcaChannelId2Bytes")
        }
        NetscalerIPFixField::NetscalerIcaChannelId3 => Ok("nf.data.netscalerIcaChannelId3"),
        NetscalerIPFixField::NetscalerIcaChannelId3Bytes => {
            Ok("nf.data.netscalerIcaChannelId3Bytes")
        }
        NetscalerIPFixField::NetscalerIcaChannelId4 => Ok("nf.data.netscalerIcaChannelId4"),
        NetscalerIPFixField::NetscalerIcaChannelId4Bytes => {
            Ok("nf.data.netscalerIcaChannelId4Bytes")
        }
        NetscalerIPFixField::NetscalerIcaChannelId5 => Ok("nf.data.netscalerIcaChannelId5"),
        NetscalerIPFixField::NetscalerIcaChannelId5Bytes => {
            Ok("nf.data.netscalerIcaChannelId5Bytes")
        }
        NetscalerIPFixField::NetscalerIcaConnectionPriority => {
            Ok("nf.data.netscalerIcaConnectionPriority")
        }
        NetscalerIPFixField::NetscalerApplicationStartupDuration => {
            Ok("nf.data.netscalerApplicationStartupDuration")
        }
        NetscalerIPFixField::NetscalerIcaLaunchMechanism => {
            Ok("nf.data.netscalerIcaLaunchMechanism")
        }
        NetscalerIPFixField::NetscalerIcaApplicationName => {
            Ok("nf.data.netscalerIcaApplicationName")
        }
        NetscalerIPFixField::NetscalerApplicationStartupTime => {
            Ok("nf.data.netscalerApplicationStartupTime")
        }
        NetscalerIPFixField::NetscalerIcaApplicationTerminationType => {
            Ok("nf.data.netscalerIcaApplicationTerminationType")
        }
        NetscalerIPFixField::NetscalerIcaApplicationTerminationTime => {
            Ok("nf.data.netscalerIcaApplicationTerminationTime")
        }
        NetscalerIPFixField::NetscalerIcaSessionEndTime => Ok("nf.data.netscalerIcaSessionEndTime"),
        NetscalerIPFixField::NetscalerIcaClientsideJitter => {
            Ok("nf.data.netscalerIcaClientsideJitter")
        }
        NetscalerIPFixField::NetscalerIcaServersideJitter => {
            Ok("nf.data.netscalerIcaServersideJitter")
        }
        NetscalerIPFixField::NetscalerIcaAppProcessID => Ok("nf.data.netscalerIcaAppProcessID"),
        NetscalerIPFixField::NetscalerIcaAppModulePath => Ok("nf.data.netscalerIcaAppModulePath"),
        NetscalerIPFixField::NetscalerIcaDeviceSerialNo => Ok("nf.data.netscalerIcaDeviceSerialNo"),
        NetscalerIPFixField::NetscalerMsiClientCookie => Ok("nf.data.netscalerMsiClientCookie"),
        NetscalerIPFixField::NetscalerIcaFlags => Ok("nf.data.netscalerIcaFlags"),
        NetscalerIPFixField::NetscalerIcaUsername => Ok("nf.data.netscalerIcaUsername"),
        NetscalerIPFixField::NetscalerLicenseType => Ok("nf.data.netscalerLicenseType"),
        NetscalerIPFixField::NetscalerMaxLicenseCount => Ok("nf.data.netscalerMaxLicenseCount"),
        NetscalerIPFixField::NetscalerCurrentLicenseConsumed => {
            Ok("nf.data.netscalerCurrentLicenseConsumed")
        }
        NetscalerIPFixField::NetscalerIcaNetworkUpdateStartTime => {
            Ok("nf.data.netscalerIcaNetworkUpdateStartTime")
        }
        NetscalerIPFixField::NetscalerIcaNetworkUpdateEndTime => {
            Ok("nf.data.netscalerIcaNetworkUpdateEndTime")
        }
        NetscalerIPFixField::NetscalerIcaClientsideSRTT => Ok("nf.data.netscalerIcaClientsideSRTT"),
        NetscalerIPFixField::NetscalerIcaServersideSRTT => Ok("nf.data.netscalerIcaServersideSRTT"),
        NetscalerIPFixField::NetscalerIcaClientsideDelay => {
            Ok("nf.data.netscalerIcaClientsideDelay")
        }
        NetscalerIPFixField::NetscalerIcaServersideDelay => {
            Ok("nf.data.netscalerIcaServersideDelay")
        }
        NetscalerIPFixField::NetscalerIcaHostDelay => Ok("nf.data.netscalerIcaHostDelay"),
        NetscalerIPFixField::NetscalerIcaClientSideWindowSize => {
            Ok("nf.data.netscalerIcaClientSideWindowSize")
        }
        NetscalerIPFixField::NetscalerIcaServerSideWindowSize => {
            Ok("nf.data.netscalerIcaServerSideWindowSize")
        }
        NetscalerIPFixField::NetscalerIcaClientSideRTOCount => {
            Ok("nf.data.netscalerIcaClientSideRTOCount")
        }
        NetscalerIPFixField::NetscalerIcaServerSideRTOCount => {
            Ok("nf.data.netscalerIcaServerSideRTOCount")
        }
        NetscalerIPFixField::NetscalerIcaL7ClientLatency => {
            Ok("nf.data.netscalerIcaL7ClientLatency")
        }
        NetscalerIPFixField::NetscalerIcaL7ServerLatency => {
            Ok("nf.data.netscalerIcaL7ServerLatency")
        }
        NetscalerIPFixField::NetscalerHttpDomainName => Ok("nf.data.netscalerHttpDomainName"),
        NetscalerIPFixField::NetscalerCacheRedirClientConnectionCoreID => {
            Ok("nf.data.netscalerCacheRedirClientConnectionCoreID")
        }
        NetscalerIPFixField::NetscalerCacheRedirClientConnectionTransactionID => {
            Ok("nf.data.netscalerCacheRedirClientConnectionTransactionID")
        }
        NetscalerIPFixField::NetscalerUnknown270 => Err(270),
        NetscalerIPFixField::NetscalerUnknown271 => Err(271),
        NetscalerIPFixField::NetscalerUnknown272 => Err(272),
        NetscalerIPFixField::NetscalerUnknown273 => Err(273),
        NetscalerIPFixField::NetscalerUnknown274 => Err(274),
        NetscalerIPFixField::NetscalerUnknown275 => Err(275),
        NetscalerIPFixField::NetscalerUnknown276 => Err(276),
        NetscalerIPFixField::NetscalerUnknown277 => Err(277),
        NetscalerIPFixField::NetscalerUnknown278 => Err(278),
        NetscalerIPFixField::NetscalerUnknown279 => Err(279),
        NetscalerIPFixField::NetscalerUnknown280 => Err(280),
        NetscalerIPFixField::NetscalerUnknown281 => Err(281),
        NetscalerIPFixField::NetscalerUnknown282 => Err(282),
        NetscalerIPFixField::NetscalerUnknown283 => Err(283),
        NetscalerIPFixField::NetscalerUnknown284 => Err(284),
        NetscalerIPFixField::NetscalerUnknown285 => Err(285),
        NetscalerIPFixField::NetscalerUnknown286 => Err(286),
        NetscalerIPFixField::NetscalerUnknown287 => Err(287),
        NetscalerIPFixField::NetscalerUnknown288 => Err(288),
        NetscalerIPFixField::NetscalerUnknown289 => Err(289),
        NetscalerIPFixField::NetscalerUnknown290 => Err(290),
        NetscalerIPFixField::NetscalerUnknown291 => Err(291),
        NetscalerIPFixField::NetscalerUnknown292 => Err(292),
        NetscalerIPFixField::NetscalerUnknown293 => Err(293),
        NetscalerIPFixField::NetscalerUnknown294 => Err(294),
        NetscalerIPFixField::NetscalerUnknown295 => Err(295),
        NetscalerIPFixField::NetscalerUnknown296 => Err(296),
        NetscalerIPFixField::NetscalerUnknown297 => Err(297),
        NetscalerIPFixField::NetscalerUnknown298 => Err(298),
        NetscalerIPFixField::NetscalerUnknown299 => Err(299),
        NetscalerIPFixField::NetscalerUnknown300 => Err(300),
        NetscalerIPFixField::NetscalerUnknown301 => Err(301),
        NetscalerIPFixField::NetscalerUnknown302 => Err(302),
        NetscalerIPFixField::NetscalerUnknown303 => Err(303),
        NetscalerIPFixField::NetscalerUnknown304 => Err(304),
        NetscalerIPFixField::NetscalerUnknown305 => Err(305),
        NetscalerIPFixField::NetscalerUnknown306 => Err(306),
        NetscalerIPFixField::NetscalerUnknown307 => Err(307),
        NetscalerIPFixField::NetscalerUnknown308 => Err(308),
        NetscalerIPFixField::NetscalerUnknown309 => Err(309),
        NetscalerIPFixField::NetscalerUnknown310 => Err(310),
        NetscalerIPFixField::NetscalerUnknown311 => Err(311),
        NetscalerIPFixField::NetscalerUnknown312 => Err(312),
        NetscalerIPFixField::NetscalerUnknown313 => Err(313),
        NetscalerIPFixField::NetscalerUnknown314 => Err(314),
        NetscalerIPFixField::NetscalerUnknown315 => Err(315),
        NetscalerIPFixField::NetscalerUnknown316 => Err(316),
        NetscalerIPFixField::NetscalerUnknown317 => Err(317),
        NetscalerIPFixField::NetscalerUnknown318 => Err(318),
        NetscalerIPFixField::NetscalerUnknown319 => Err(319),
        NetscalerIPFixField::NetscalerUnknown320 => Err(320),
        NetscalerIPFixField::NetscalerUnknown321 => Err(321),
        NetscalerIPFixField::NetscalerUnknown322 => Err(322),
        NetscalerIPFixField::NetscalerUnknown323 => Err(323),
        NetscalerIPFixField::NetscalerUnknown324 => Err(324),
        NetscalerIPFixField::NetscalerUnknown325 => Err(325),
        NetscalerIPFixField::NetscalerUnknown326 => Err(326),
        NetscalerIPFixField::NetscalerUnknown327 => Err(327),
        NetscalerIPFixField::NetscalerUnknown328 => Err(328),
        NetscalerIPFixField::NetscalerUnknown329 => Err(329),
        NetscalerIPFixField::NetscalerUnknown330 => Err(330),
        NetscalerIPFixField::NetscalerUnknown331 => Err(331),
        NetscalerIPFixField::NetscalerUnknown332 => Err(332),
        NetscalerIPFixField::NetscalerUnknown333 => Err(333),
        NetscalerIPFixField::NetscalerUnknown334 => Err(334),
        NetscalerIPFixField::NetscalerUnknown335 => Err(335),
        NetscalerIPFixField::NetscalerUnknown336 => Err(336),
        NetscalerIPFixField::NetscalerUnknown337 => Err(337),
        NetscalerIPFixField::NetscalerUnknown338 => Err(338),
        NetscalerIPFixField::NetscalerUnknown339 => Err(339),
        NetscalerIPFixField::NetscalerUnknown340 => Err(340),
        NetscalerIPFixField::NetscalerUnknown341 => Err(341),
        NetscalerIPFixField::NetscalerUnknown342 => Err(342),
        NetscalerIPFixField::NetscalerUnknown343 => Err(343),
        NetscalerIPFixField::NetscalerUnknown344 => Err(344),
        NetscalerIPFixField::NetscalerUnknown345 => Err(345),
        NetscalerIPFixField::NetscalerUnknown346 => Err(346),
        NetscalerIPFixField::NetscalerUnknown347 => Err(347),
        NetscalerIPFixField::NetscalerUnknown348 => Err(348),
        NetscalerIPFixField::NetscalerUnknown349 => Err(349),
        NetscalerIPFixField::NetscalerUnknown350 => Err(350),
        NetscalerIPFixField::NetscalerUnknown351 => Err(351),
        NetscalerIPFixField::NetscalerUnknown352 => Err(352),
        NetscalerIPFixField::NetscalerUnknown353 => Err(353),
        NetscalerIPFixField::NetscalerUnknown354 => Err(354),
        NetscalerIPFixField::NetscalerUnknown355 => Err(355),
        NetscalerIPFixField::NetscalerUnknown356 => Err(356),
        NetscalerIPFixField::NetscalerUnknown357 => Err(357),
        NetscalerIPFixField::NetscalerUnknown358 => Err(358),
        NetscalerIPFixField::NetscalerUnknown359 => Err(359),
        NetscalerIPFixField::NetscalerUnknown360 => Err(360),
        NetscalerIPFixField::NetscalerUnknown361 => Err(361),
        NetscalerIPFixField::NetscalerUnknown362 => Err(362),
        NetscalerIPFixField::NetscalerUnknown363 => Err(363),
        NetscalerIPFixField::NetscalerUnknown364 => Err(364),
        NetscalerIPFixField::NetscalerUnknown365 => Err(365),
        NetscalerIPFixField::NetscalerUnknown366 => Err(366),
        NetscalerIPFixField::NetscalerUnknown367 => Err(367),
        NetscalerIPFixField::NetscalerUnknown377 => Err(377),
        NetscalerIPFixField::NetscalerUnknown380 => Err(380),
        NetscalerIPFixField::NetscalerUnknown383 => Err(383),
        NetscalerIPFixField::NetscalerUnknown384 => Err(384),
        NetscalerIPFixField::NetscalerUnknown385 => Err(385),
        NetscalerIPFixField::NetscalerUnknown386 => Err(386),
        NetscalerIPFixField::NetscalerUnknown387 => Err(387),
        NetscalerIPFixField::NetscalerUnknown388 => Err(388),
        NetscalerIPFixField::NetscalerUnknown389 => Err(389),
        NetscalerIPFixField::NetscalerUnknown391 => Err(391),
        NetscalerIPFixField::NetscalerUnknown392 => Err(392),
        NetscalerIPFixField::NetscalerUnknown393 => Err(393),
        NetscalerIPFixField::NetscalerUnknown394 => Err(394),
        NetscalerIPFixField::NetscalerUnknown398 => Err(398),
        NetscalerIPFixField::NetscalerUnknown399 => Err(399),
        NetscalerIPFixField::NetscalerUnknown400 => Err(400),
        NetscalerIPFixField::NetscalerUnknown401 => Err(401),
        NetscalerIPFixField::NetscalerUnknown402 => Err(402),
        NetscalerIPFixField::NetscalerUnknown403 => Err(403),
        NetscalerIPFixField::NetscalerUnknown404 => Err(404),
        NetscalerIPFixField::NetscalerUnknown405 => Err(405),
        NetscalerIPFixField::NetscalerUnknown408 => Err(408),
        NetscalerIPFixField::NetscalerUnknown427 => Err(427),
        NetscalerIPFixField::NetscalerUnknown429 => Err(429),
        NetscalerIPFixField::NetscalerUnknown432 => Err(432),
        NetscalerIPFixField::NetscalerUnknown433 => Err(433),
        NetscalerIPFixField::NetscalerUnknown434 => Err(434),
        NetscalerIPFixField::NetscalerUnknown435 => Err(435),
        NetscalerIPFixField::NetscalerUnknown447 => Err(447),
        NetscalerIPFixField::NetscalerUnknown448 => Err(448),
        NetscalerIPFixField::NetscalerUnknown451 => Err(451),
        NetscalerIPFixField::NetscalerUnknown452 => Err(452),
        NetscalerIPFixField::NetscalerUnknown453 => Err(453),
        NetscalerIPFixField::NetscalerUnknown455 => Err(455),
        NetscalerIPFixField::NetscalerUnknown458 => Err(458),
        NetscalerIPFixField::NetscalerUnknown459 => Err(459),
        NetscalerIPFixField::NetscalerUnknown460 => Err(460),
        NetscalerIPFixField::NetscalerUnknown461 => Err(461),
        NetscalerIPFixField::NetscalerUnknown463 => Err(463),
        NetscalerIPFixField::NetscalerUnknown464 => Err(464),
        NetscalerIPFixField::NetscalerUnknown465 => Err(465),
        NetscalerIPFixField::NetscalerUnknown466 => Err(466),
        NetscalerIPFixField::NetscalerUnknown467 => Err(467),
        NetscalerIPFixField::NetscalerUnknown468 => Err(468),
        NetscalerIPFixField::NetscalerUnknown475 => Err(475),
        NetscalerIPFixField::NetscalerUnknown476 => Err(476),
        NetscalerIPFixField::NetscalerUnknown477 => Err(477),
        NetscalerIPFixField::NetscalerUnknown478 => Err(478),
        NetscalerIPFixField::NetscalerUnknown479 => Err(479),
        NetscalerIPFixField::NetscalerUnknown480 => Err(480),
        NetscalerIPFixField::NetscalerUnknown481 => Err(481),
        NetscalerIPFixField::NetscalerUnknown487 => Err(487),
        NetscalerIPFixField::NetscalerUnknown488 => Err(488),
        NetscalerIPFixField::NetscalerUnknown489 => Err(489),
        NetscalerIPFixField::NetscalerUnknown491 => Err(491),
        NetscalerIPFixField::NetscalerUnknown492 => Err(492),
        NetscalerIPFixField::NetscalerUnknown493 => Err(493),
        NetscalerIPFixField::NetscalerUnknown494 => Err(494),
        NetscalerIPFixField::NetscalerUnknown507 => Err(507),
        NetscalerIPFixField::NetscalerUnknown508 => Err(508),
        NetscalerIPFixField::NetscalerUnknown509 => Err(509),
        NetscalerIPFixField::NetscalerUnknown510 => Err(510),
        NetscalerIPFixField::NetscalerUnknown511 => Err(511),
        NetscalerIPFixField::NetscalerUnknown512 => Err(512),
        NetscalerIPFixField::NetscalerUnknown513 => Err(513),
        NetscalerIPFixField::NetscalerUnknown514 => Err(514),
        NetscalerIPFixField::NetscalerUnknown518 => Err(518),
        NetscalerIPFixField::NetscalerUnknown519 => Err(519),
        NetscalerIPFixField::NetscalerUnknown520 => Err(520),
        NetscalerIPFixField::NetscalerUnknown521 => Err(521),
        NetscalerIPFixField::NetscalerUnknown522 => Err(522),
        NetscalerIPFixField::NetscalerUnknown523 => Err(523),
        NetscalerIPFixField::NetscalerUnknown532 => Err(532),
        NetscalerIPFixField::NetscalerUnknown533 => Err(533),
        NetscalerIPFixField::NetscalerUnknown534 => Err(534),
        NetscalerIPFixField::NetscalerUnknown535 => Err(535),
        NetscalerIPFixField::NetscalerUnknown536 => Err(536),
        NetscalerIPFixField::NetscalerUnknown537 => Err(537),
        NetscalerIPFixField::NetscalerUnknown538 => Err(538),
        NetscalerIPFixField::NetscalerUnknown541 => Err(541),
        NetscalerIPFixField::Unknown(field_num) => Err(*field_num),
    }
}

pub fn ipfix_nat_field_map(field: &NatIPFixField) -> Result<&'static str, u16> {
    match field {
        NatIPFixField::NatInsideSvcid => Ok("nf.data.natInsideSvcid"),
        NatIPFixField::NatOutsideSvcid => Ok("nf.data.natOutsideSvcid"),
        NatIPFixField::NatSubString => Ok("nf.data.natSubString"),
        NatIPFixField::Unknown(field_num) => Err(*field_num),
    }
}

pub fn ipfix_yaf_field_map(field: &YafIPFixField) -> Result<&'static str, u16> {
    match field {
        YafIPFixField::InitialTCPFlags => Ok("nf.data.initialTCPFlags"),
        YafIPFixField::UnionTCPFlags => Ok("nf.data.unionTCPFlags"),
        YafIPFixField::Payload => Ok("nf.data.payload"),
        YafIPFixField::ReverseFlowDeltaMilliseconds => Ok("nf.data.reverseFlowDeltaMilliseconds"),
        YafIPFixField::SilkAppLabel => Ok("nf.data.silkAppLabel"),
        YafIPFixField::PayloadEntropy => Ok("nf.data.payloadEntropy"),
        YafIPFixField::OsName => Ok("nf.data.osName"),
        YafIPFixField::OsVersion => Ok("nf.data.osVersion"),
        YafIPFixField::FirstPacketBanner => Ok("nf.data.firstPacketBanner"),
        YafIPFixField::SecondPacketBanner => Ok("nf.data.secondPacketBanner"),
        YafIPFixField::FlowAttributes => Ok("nf.data.flowAttributes"),
        YafIPFixField::ExpiredFragmentCount => Ok("nf.data.expiredFragmentCount"),
        YafIPFixField::AssembledFragmentCount => Ok("nf.data.assembledFragmentCount"),
        YafIPFixField::MeanFlowRate => Ok("nf.data.meanFlowRate"),
        YafIPFixField::MeanPacketRate => Ok("nf.data.meanPacketRate"),
        YafIPFixField::FlowTableFlushEventCount => Ok("nf.data.flowTableFlushEventCount"),
        YafIPFixField::FlowTablePeakCount => Ok("nf.data.flowTablePeakCount"),
        YafIPFixField::OsFingerPrint => Ok("nf.data.osFingerPrint"),
        YafIPFixField::TftpFilename => Ok("nf.data.tftpFilename"),
        YafIPFixField::TftpMode => Ok("nf.data.tftpMode"),
        YafIPFixField::DnsQueryResponse => Ok("nf.data.dnsQueryResponse"),
        YafIPFixField::DnsQRType => Ok("nf.data.dnsQRType"),
        YafIPFixField::DnsAuthoritative => Ok("nf.data.dnsAuthoritative"),
        YafIPFixField::DnsNXDomain => Ok("nf.data.dnsNXDomain"),
        YafIPFixField::DnsRRSection => Ok("nf.data.dnsRRSection"),
        YafIPFixField::DnsQName => Ok("nf.data.dnsQName"),
        YafIPFixField::DnsCName => Ok("nf.data.dnsCName"),
        YafIPFixField::DnsMXPreference => Ok("nf.data.dnsMXPreference"),
        YafIPFixField::DnsMXExchange => Ok("nf.data.dnsMXExchange"),
        YafIPFixField::DnsNSDName => Ok("nf.data.dnsNSDName"),
        YafIPFixField::DnsPTRDName => Ok("nf.data.dnsPTRDName"),
        YafIPFixField::SslCipher => Ok("nf.data.sslCipher"),
        YafIPFixField::SslClientVersion => Ok("nf.data.sslClientVersion"),
        YafIPFixField::SslServerCipher => Ok("nf.data.sslServerCipher"),
        YafIPFixField::SslCompressionMethod => Ok("nf.data.sslCompressionMethod"),
        YafIPFixField::SslCertVersion => Ok("nf.data.sslCertVersion"),
        YafIPFixField::SslCertSignature => Ok("nf.data.sslCertSignature"),
        YafIPFixField::DnsTTL => Ok("nf.data.dnsTTL"),
        YafIPFixField::DnsTXTData => Ok("nf.data.dnsTXTData"),
        YafIPFixField::DnsSOASerial => Ok("nf.data.dnsSOASerial"),
        YafIPFixField::DnsSOARefresh => Ok("nf.data.dnsSOARefresh"),
        YafIPFixField::DnsSOARetry => Ok("nf.data.dnsSOARetry"),
        YafIPFixField::DnsSOAExpire => Ok("nf.data.dnsSOAExpire"),
        YafIPFixField::DnsSOAMinimum => Ok("nf.data.dnsSOAMinimum"),
        YafIPFixField::DnsSOAMName => Ok("nf.data.dnsSOAMName"),
        YafIPFixField::DnsSOARName => Ok("nf.data.dnsSOARName"),
        YafIPFixField::DnsSRVPriority => Ok("nf.data.dnsSRVPriority"),
        YafIPFixField::DnsSRVWeight => Ok("nf.data.dnsSRVWeight"),
        YafIPFixField::DnsSRVPort => Ok("nf.data.dnsSRVPort"),
        YafIPFixField::DnsSRVTarget => Ok("nf.data.dnsSRVTarget"),
        YafIPFixField::TcpUrgTotalCount => Ok("nf.data.tcpUrgTotalCount"),
        YafIPFixField::DnsID => Ok("nf.data.dnsID"),
        YafIPFixField::SslCertSerialNumber => Ok("nf.data.sslCertSerialNumber"),
        YafIPFixField::SslObjectType => Ok("nf.data.sslObjectType"),
        YafIPFixField::SslObjectValue => Ok("nf.data.sslObjectValue"),
        YafIPFixField::SslCertValidityNotBefore => Ok("nf.data.sslCertValidityNotBefore"),
        YafIPFixField::SslCertValidityNotAfter => Ok("nf.data.sslCertValidityNotAfter"),
        YafIPFixField::SslPublicKeyAlgorithm => Ok("nf.data.sslPublicKeyAlgorithm"),
        YafIPFixField::SslPublicKeyLength => Ok("nf.data.sslPublicKeyLength"),
        YafIPFixField::RtpPayloadType => Ok("nf.data.rtpPayloadType"),
        YafIPFixField::ReverseRtpPayloadType => Ok("nf.data.reverseRtpPayloadType"),
        YafIPFixField::MptcpInitialDataSequenceNumber => {
            Ok("nf.data.mptcpInitialDataSequenceNumber")
        }
        YafIPFixField::MptcpReceiverToken => Ok("nf.data.mptcpReceiverToken"),
        YafIPFixField::MptcpMaximumSegmentSize => Ok("nf.data.mptcpMaximumSegmentSize"),
        YafIPFixField::MptcpAddressID => Ok("nf.data.mptcpAddressID"),
        YafIPFixField::MptcpFlags => Ok("nf.data.mptcpFlags"),
        YafIPFixField::SslServerName => Ok("nf.data.sslServerName"),
        YafIPFixField::SslCertificateHash => Ok("nf.data.sslCertificateHash"),
        YafIPFixField::SmallPacketCount => Ok("nf.data.smallPacketCount"),
        YafIPFixField::NonEmptyPacketCount => Ok("nf.data.nonEmptyPacketCount"),
        YafIPFixField::DataByteCount => Ok("nf.data.dataByteCount"),
        YafIPFixField::AverageInterarrivalTime => Ok("nf.data.averageInterarrivalTime"),
        YafIPFixField::StandardDeviationInterarrivalTime => {
            Ok("nf.data.standardDeviationInterarrivalTime")
        }
        YafIPFixField::FirstNonEmptyPacketSize => Ok("nf.data.firstNonEmptyPacketSize"),
        YafIPFixField::MaxPacketSize => Ok("nf.data.maxPacketSize"),
        YafIPFixField::FirstEightNonEmptyPacketDirections => {
            Ok("nf.data.firstEightNonEmptyPacketDirections")
        }
        YafIPFixField::StandardDeviationPayloadLength => {
            Ok("nf.data.standardDeviationPayloadLength")
        }
        YafIPFixField::LargePacketCount => Ok("nf.data.largePacketCount"),
        YafIPFixField::ReverseInitialTCPFlags => Ok("nf.data.reverseInitialTCPFlags"),
        YafIPFixField::ReverseUnionTCPFlags => Ok("nf.data.reverseUnionTCPFlags"),
        YafIPFixField::ReversePayload => Ok("nf.data.reversePayload"),
        YafIPFixField::ReversePayloadEntropy => Ok("nf.data.reversePayloadEntropy"),
        YafIPFixField::ReverseOsName => Ok("nf.data.reverseOsName"),
        YafIPFixField::ReverseOsVersion => Ok("nf.data.reverseOsVersion"),
        YafIPFixField::ReverseFirstPacketBanner => Ok("nf.data.reverseFirstPacketBanner"),
        YafIPFixField::ReverseSecondPacketBanner => Ok("nf.data.reverseSecondPacketBanner"),
        YafIPFixField::ReverseFlowAttributes => Ok("nf.data.reverseFlowAttributes"),
        YafIPFixField::ReverseOsFingerPrint => Ok("nf.data.reverseOsFingerPrint"),
        YafIPFixField::ReverseSmallPacketCount => Ok("nf.data.reverseSmallPacketCount"),
        YafIPFixField::ReverseNonEmptyPacketCount => Ok("nf.data.reverseNonEmptyPacketCount"),
        YafIPFixField::ReverseDataByteCount => Ok("nf.data.reverseDataByteCount"),
        YafIPFixField::ReverseAverageInterarrivalTime => {
            Ok("nf.data.reverseAverageInterarrivalTime")
        }
        YafIPFixField::ReverseStandardDeviationInterarrivalTime => {
            Ok("nf.data.reverseStandardDeviationInterarrivalTime")
        }
        YafIPFixField::ReverseFirstNonEmptyPacketSize => {
            Ok("nf.data.reverseFirstNonEmptyPacketSize")
        }
        YafIPFixField::ReverseMaxPacketSize => Ok("nf.data.reverseMaxPacketSize"),
        YafIPFixField::ReverseStandardDeviationPayloadLength => {
            Ok("nf.data.reverseStandardDeviationPayloadLength")
        }
        YafIPFixField::ReverseLargePacketCount => Ok("nf.data.reverseLargePacketCount"),
        YafIPFixField::Unknown(field_num) => Err(*field_num),
    }
}

pub fn ipfix_vmware_field_map(field: &VMWareIPFixField) -> Result<&'static str, u16> {
    match field {
        VMWareIPFixField::VmwareTenantProtocol => Ok("nf.data.vmwareTenantProtocol"),
        VMWareIPFixField::VmwareTenantSourceIPv4 => Ok("nf.data.vmwareTenantSourceIPv4"),
        VMWareIPFixField::VmwareTenantDestIPv4 => Ok("nf.data.vmwareTenantDestIPv4"),
        VMWareIPFixField::VmwareTenantSourceIPv6 => Ok("nf.data.vmwareTenantSourceIPv6"),
        VMWareIPFixField::VmwareTenantDestIPv6 => Ok("nf.data.vmwareTenantDestIPv6"),
        VMWareIPFixField::VmwareTenantSourcePort => Ok("nf.data.vmwareTenantSourcePort"),
        VMWareIPFixField::VmwareTenantDestPort => Ok("nf.data.vmwareTenantDestPort"),
        VMWareIPFixField::VmwareEgressInterfaceAttr => Ok("nf.data.vmwareEgressInterfaceAttr"),
        VMWareIPFixField::VmwareVxlanExportRole => Ok("nf.data.vmwareVxlanExportRole"),
        VMWareIPFixField::VmwareIngressInterfaceAttr => Ok("nf.data.vmwareIngressInterfaceAttr"),
        VMWareIPFixField::Unknown(field_num) => Err(*field_num),
    }
}

pub fn ipfix_reverse_field_map(field: &ReverseInformationElement) -> Result<&'static str, u16> {
    match field {
        ReverseInformationElement::ReverseOctetDeltaCount => Ok("nf.data.reverseOctetDeltaCount"),
        ReverseInformationElement::ReversePacketDeltaCount => Ok("nf.data.reversePacketDeltaCount"),
        ReverseInformationElement::ReverseDeltaFlowCount => Ok("nf.data.reverseDeltaFlowCount"),
        ReverseInformationElement::ReverseProtocolIdentifier => {
            Ok("nf.data.reverseProtocolIdentifier")
        }
        ReverseInformationElement::ReverseIpClassOfService => Ok("nf.data.reverseIpClassOfService"),
        ReverseInformationElement::ReverseTcpControlBits => Ok("nf.data.reverseTcpControlBits"),
        ReverseInformationElement::ReverseSourceTransportPort => {
            Ok("nf.data.reverseSourceTransportPort")
        }
        ReverseInformationElement::ReverseSourceIPv4Address => {
            Ok("nf.data.reverseSourceIPv4Address")
        }
        ReverseInformationElement::ReverseSourceIPv4PrefixLength => {
            Ok("nf.data.reverseSourceIPv4PrefixLength")
        }
        ReverseInformationElement::ReverseIngressInterface => Ok("nf.data.reverseIngressInterface"),
        ReverseInformationElement::ReverseDestinationTransportPort => {
            Ok("nf.data.reverseDestinationTransportPort")
        }
        ReverseInformationElement::ReverseDestinationIPv4Address => {
            Ok("nf.data.reverseDestinationIPv4Address")
        }
        ReverseInformationElement::ReverseDestinationIPv4PrefixLength => {
            Ok("nf.data.reverseDestinationIPv4PrefixLength")
        }
        ReverseInformationElement::ReverseEgressInterface => Ok("nf.data.reverseEgressInterface"),
        ReverseInformationElement::ReverseIpNextHopIPv4Address => {
            Ok("nf.data.reverseIpNextHopIPv4Address")
        }
        ReverseInformationElement::ReverseBgpSourceAsNumber => {
            Ok("nf.data.reverseBgpSourceAsNumber")
        }
        ReverseInformationElement::ReverseBgpDestinationAsNumber => {
            Ok("nf.data.reverseBgpDestinationAsNumber")
        }
        ReverseInformationElement::ReverseBgpNextHopIPv4Address => {
            Ok("nf.data.reverseBgpNextHopIPv4Address")
        }
        ReverseInformationElement::ReversePostMCastPacketDeltaCount => {
            Ok("nf.data.reversePostMCastPacketDeltaCount")
        }
        ReverseInformationElement::ReversePostMCastOctetDeltaCount => {
            Ok("nf.data.reversePostMCastOctetDeltaCount")
        }
        ReverseInformationElement::ReverseFlowEndSysUpTime => Ok("nf.data.reverseFlowEndSysUpTime"),
        ReverseInformationElement::ReverseFlowStartSysUpTime => {
            Ok("nf.data.reverseFlowStartSysUpTime")
        }
        ReverseInformationElement::ReversePostOctetDeltaCount => {
            Ok("nf.data.reversePostOctetDeltaCount")
        }
        ReverseInformationElement::ReversePostPacketDeltaCount => {
            Ok("nf.data.reversePostPacketDeltaCount")
        }
        ReverseInformationElement::ReverseMinimumIpTotalLength => {
            Ok("nf.data.reverseMinimumIpTotalLength")
        }
        ReverseInformationElement::ReverseMaximumIpTotalLength => {
            Ok("nf.data.reverseMaximumIpTotalLength")
        }
        ReverseInformationElement::ReverseSourceIPv6Address => {
            Ok("nf.data.reverseSourceIPv6Address")
        }
        ReverseInformationElement::ReverseDestinationIPv6Address => {
            Ok("nf.data.reverseDestinationIPv6Address")
        }
        ReverseInformationElement::ReverseSourceIPv6PrefixLength => {
            Ok("nf.data.reverseSourceIPv6PrefixLength")
        }
        ReverseInformationElement::ReverseDestinationIPv6PrefixLength => {
            Ok("nf.data.reverseDestinationIPv6PrefixLength")
        }
        ReverseInformationElement::ReverseFlowLabelIPv6 => Ok("nf.data.reverseFlowLabelIPv6"),
        ReverseInformationElement::ReverseIcmpTypeCodeIPv4 => Ok("nf.data.reverseIcmpTypeCodeIPv4"),
        ReverseInformationElement::ReverseIgmpType => Ok("nf.data.reverseIgmpType"),
        ReverseInformationElement::ReverseSamplingInterval => Ok("nf.data.reverseSamplingInterval"),
        ReverseInformationElement::ReverseSamplingAlgorithm => {
            Ok("nf.data.reverseSamplingAlgorithm")
        }
        ReverseInformationElement::ReverseFlowActiveTimeout => {
            Ok("nf.data.reverseFlowActiveTimeout")
        }
        ReverseInformationElement::ReverseFlowIdleTimeout => Ok("nf.data.reverseFlowIdleTimeout"),
        ReverseInformationElement::ReverseEngineType => Ok("nf.data.reverseEngineType"),
        ReverseInformationElement::ReverseEngineId => Ok("nf.data.reverseEngineId"),
        ReverseInformationElement::ReverseIpv4RouterSc => Ok("nf.data.reverseIpv4RouterSc"),
        ReverseInformationElement::ReverseSourceIPv4Prefix => Ok("nf.data.reverseSourceIPv4Prefix"),
        ReverseInformationElement::ReverseDestinationIPv4Prefix => {
            Ok("nf.data.reverseDestinationIPv4Prefix")
        }
        ReverseInformationElement::ReverseMplsTopLabelType => Ok("nf.data.reverseMplsTopLabelType"),
        ReverseInformationElement::ReverseMplsTopLabelIPv4Address => {
            Ok("nf.data.reverseMplsTopLabelIPv4Address")
        }
        ReverseInformationElement::ReverseSamplerId => Ok("nf.data.reverseSamplerId"),
        ReverseInformationElement::ReverseSamplerMode => Ok("nf.data.reverseSamplerMode"),
        ReverseInformationElement::ReverseSamplerRandomInterval => {
            Ok("nf.data.reverseSamplerRandomInterval")
        }
        ReverseInformationElement::ReverseClassId => Ok("nf.data.reverseClassId"),
        ReverseInformationElement::ReverseMinimumTTL => Ok("nf.data.reverseMinimumTTL"),
        ReverseInformationElement::ReverseMaximumTTL => Ok("nf.data.reverseMaximumTTL"),
        ReverseInformationElement::ReverseFragmentIdentification => {
            Ok("nf.data.reverseFragmentIdentification")
        }
        ReverseInformationElement::ReversePostIpClassOfService => {
            Ok("nf.data.reversePostIpClassOfService")
        }
        ReverseInformationElement::ReverseSourceMacAddress => Ok("nf.data.reverseSourceMacAddress"),
        ReverseInformationElement::ReversePostDestinationMacAddress => {
            Ok("nf.data.reversePostDestinationMacAddress")
        }
        ReverseInformationElement::ReverseVlanId => Ok("nf.data.reverseVlanId"),
        ReverseInformationElement::ReversePostVlanId => Ok("nf.data.reversePostVlanId"),
        ReverseInformationElement::ReverseIpVersion => Ok("nf.data.reverseIpVersion"),
        ReverseInformationElement::ReverseFlowDirection => Ok("nf.data.reverseFlowDirection"),
        ReverseInformationElement::ReverseIpNextHopIPv6Address => {
            Ok("nf.data.reverseIpNextHopIPv6Address")
        }
        ReverseInformationElement::ReverseBgpNextHopIPv6Address => {
            Ok("nf.data.reverseBgpNextHopIPv6Address")
        }
        ReverseInformationElement::ReverseIpv6ExtensionHeaders => {
            Ok("nf.data.reverseIpv6ExtensionHeaders")
        }
        ReverseInformationElement::ReverseMplsTopLabelStackSection => {
            Ok("nf.data.reverseMplsTopLabelStackSection")
        }
        ReverseInformationElement::ReverseMplsLabelStackSection2 => {
            Ok("nf.data.reverseMplsLabelStackSection2")
        }
        ReverseInformationElement::ReverseMplsLabelStackSection3 => {
            Ok("nf.data.reverseMplsLabelStackSection3")
        }
        ReverseInformationElement::ReverseMplsLabelStackSection4 => {
            Ok("nf.data.reverseMplsLabelStackSection4")
        }
        ReverseInformationElement::ReverseMplsLabelStackSection5 => {
            Ok("nf.data.reverseMplsLabelStackSection5")
        }
        ReverseInformationElement::ReverseMplsLabelStackSection6 => {
            Ok("nf.data.reverseMplsLabelStackSection6")
        }
        ReverseInformationElement::ReverseMplsLabelStackSection7 => {
            Ok("nf.data.reverseMplsLabelStackSection7")
        }
        ReverseInformationElement::ReverseMplsLabelStackSection8 => {
            Ok("nf.data.reverseMplsLabelStackSection8")
        }
        ReverseInformationElement::ReverseMplsLabelStackSection9 => {
            Ok("nf.data.reverseMplsLabelStackSection9")
        }
        ReverseInformationElement::ReverseMplsLabelStackSection10 => {
            Ok("nf.data.reverseMplsLabelStackSection10")
        }
        ReverseInformationElement::ReverseDestinationMacAddress => {
            Ok("nf.data.reverseDestinationMacAddress")
        }
        ReverseInformationElement::ReversePostSourceMacAddress => {
            Ok("nf.data.reversePostSourceMacAddress")
        }
        ReverseInformationElement::ReverseInterfaceName => Ok("nf.data.reverseInterfaceName"),
        ReverseInformationElement::ReverseInterfaceDescription => {
            Ok("nf.data.reverseInterfaceDescription")
        }
        ReverseInformationElement::ReverseSamplerName => Ok("nf.data.reverseSamplerName"),
        ReverseInformationElement::ReverseOctetTotalCount => Ok("nf.data.reverseOctetTotalCount"),
        ReverseInformationElement::ReversePacketTotalCount => Ok("nf.data.reversePacketTotalCount"),
        ReverseInformationElement::ReverseFlagsAndSamplerId => {
            Ok("nf.data.reverseFlagsAndSamplerId")
        }
        ReverseInformationElement::ReverseFragmentOffset => Ok("nf.data.reverseFragmentOffset"),
        ReverseInformationElement::ReverseForwardingStatus => Ok("nf.data.reverseForwardingStatus"),
        ReverseInformationElement::ReverseMplsVpnRouteDistinguisher => {
            Ok("nf.data.reverseMplsVpnRouteDistinguisher")
        }
        ReverseInformationElement::ReverseMplsTopLabelPrefixLength => {
            Ok("nf.data.reverseMplsTopLabelPrefixLength")
        }
        ReverseInformationElement::ReverseSrcTrafficIndex => Ok("nf.data.reverseSrcTrafficIndex"),
        ReverseInformationElement::ReverseDstTrafficIndex => Ok("nf.data.reverseDstTrafficIndex"),
        ReverseInformationElement::ReverseApplicationDescription => {
            Ok("nf.data.reverseApplicationDescription")
        }
        ReverseInformationElement::ReverseApplicationId => Ok("nf.data.reverseApplicationId"),
        ReverseInformationElement::ReverseApplicationName => Ok("nf.data.reverseApplicationName"),
        ReverseInformationElement::ReversePostIpDiffServCodePoint => {
            Ok("nf.data.reversePostIpDiffServCodePoint")
        }
        ReverseInformationElement::ReverseMulticastReplicationFactor => {
            Ok("nf.data.reverseMulticastReplicationFactor")
        }
        ReverseInformationElement::ReverseClassName => Ok("nf.data.reverseClassName"),
        ReverseInformationElement::ReverseClassificationEngineId => {
            Ok("nf.data.reverseClassificationEngineId")
        }
        ReverseInformationElement::ReverseLayer2packetSectionOffset => {
            Ok("nf.data.reverseLayer2packetSectionOffset")
        }
        ReverseInformationElement::ReverseLayer2packetSectionSize => {
            Ok("nf.data.reverseLayer2packetSectionSize")
        }
        ReverseInformationElement::ReverseLayer2packetSectionData => {
            Ok("nf.data.reverseLayer2packetSectionData")
        }
        ReverseInformationElement::ReverseBgpNextAdjacentAsNumber => {
            Ok("nf.data.reverseBgpNextAdjacentAsNumber")
        }
        ReverseInformationElement::ReverseBgpPrevAdjacentAsNumber => {
            Ok("nf.data.reverseBgpPrevAdjacentAsNumber")
        }
        ReverseInformationElement::ReverseDroppedOctetDeltaCount => {
            Ok("nf.data.reverseDroppedOctetDeltaCount")
        }
        ReverseInformationElement::ReverseDroppedPacketDeltaCount => {
            Ok("nf.data.reverseDroppedPacketDeltaCount")
        }
        ReverseInformationElement::ReverseDroppedOctetTotalCount => {
            Ok("nf.data.reverseDroppedOctetTotalCount")
        }
        ReverseInformationElement::ReverseDroppedPacketTotalCount => {
            Ok("nf.data.reverseDroppedPacketTotalCount")
        }
        ReverseInformationElement::ReverseFlowEndReason => Ok("nf.data.reverseFlowEndReason"),
        ReverseInformationElement::ReverseObservationPointId => {
            Ok("nf.data.reverseObservationPointId")
        }
        ReverseInformationElement::ReverseIcmpTypeCodeIPv6 => Ok("nf.data.reverseIcmpTypeCodeIPv6"),
        ReverseInformationElement::ReverseMplsTopLabelIPv6Address => {
            Ok("nf.data.reverseMplsTopLabelIPv6Address")
        }
        ReverseInformationElement::ReverseLineCardId => Ok("nf.data.reverseLineCardId"),
        ReverseInformationElement::ReversePortId => Ok("nf.data.reversePortId"),
        ReverseInformationElement::ReverseMeteringProcessId => {
            Ok("nf.data.reverseMeteringProcessId")
        }
        ReverseInformationElement::ReverseExportingProcessId => {
            Ok("nf.data.reverseExportingProcessId")
        }
        ReverseInformationElement::ReverseWlanChannelId => Ok("nf.data.reverseWlanChannelId"),
        ReverseInformationElement::ReverseWlanSSID => Ok("nf.data.reverseWlanSSID"),
        ReverseInformationElement::ReverseFlowStartSeconds => Ok("nf.data.reverseFlowStartSeconds"),
        ReverseInformationElement::ReverseFlowEndSeconds => Ok("nf.data.reverseFlowEndSeconds"),
        ReverseInformationElement::ReverseFlowStartMilliseconds => {
            Ok("nf.data.reverseFlowStartMilliseconds")
        }
        ReverseInformationElement::ReverseFlowEndMilliseconds => {
            Ok("nf.data.reverseFlowEndMilliseconds")
        }
        ReverseInformationElement::ReverseFlowStartMicroseconds => {
            Ok("nf.data.reverseFlowStartMicroseconds")
        }
        ReverseInformationElement::ReverseFlowEndMicroseconds => {
            Ok("nf.data.reverseFlowEndMicroseconds")
        }
        ReverseInformationElement::ReverseFlowStartNanoseconds => {
            Ok("nf.data.reverseFlowStartNanoseconds")
        }
        ReverseInformationElement::ReverseFlowEndNanoseconds => {
            Ok("nf.data.reverseFlowEndNanoseconds")
        }
        ReverseInformationElement::ReverseFlowStartDeltaMicroseconds => {
            Ok("nf.data.reverseFlowStartDeltaMicroseconds")
        }
        ReverseInformationElement::ReverseFlowEndDeltaMicroseconds => {
            Ok("nf.data.reverseFlowEndDeltaMicroseconds")
        }
        ReverseInformationElement::ReverseSystemInitTimeMilliseconds => {
            Ok("nf.data.reverseSystemInitTimeMilliseconds")
        }
        ReverseInformationElement::ReverseFlowDurationMilliseconds => {
            Ok("nf.data.reverseFlowDurationMilliseconds")
        }
        ReverseInformationElement::ReverseFlowDurationMicroseconds => {
            Ok("nf.data.reverseFlowDurationMicroseconds")
        }
        ReverseInformationElement::ReverseDestinationIPv6Prefix => {
            Ok("nf.data.reverseDestinationIPv6Prefix")
        }
        ReverseInformationElement::ReverseSourceIPv6Prefix => Ok("nf.data.reverseSourceIPv6Prefix"),
        ReverseInformationElement::ReversePostOctetTotalCount => {
            Ok("nf.data.reversePostOctetTotalCount")
        }
        ReverseInformationElement::ReversePostPacketTotalCount => {
            Ok("nf.data.reversePostPacketTotalCount")
        }
        ReverseInformationElement::ReversePostMCastPacketTotalCount => {
            Ok("nf.data.reversePostMCastPacketTotalCount")
        }
        ReverseInformationElement::ReversePostMCastOctetTotalCount => {
            Ok("nf.data.reversePostMCastOctetTotalCount")
        }
        ReverseInformationElement::ReverseIcmpTypeIPv4 => Ok("nf.data.reverseIcmpTypeIPv4"),
        ReverseInformationElement::ReverseIcmpCodeIPv4 => Ok("nf.data.reverseIcmpCodeIPv4"),
        ReverseInformationElement::ReverseIcmpTypeIPv6 => Ok("nf.data.reverseIcmpTypeIPv6"),
        ReverseInformationElement::ReverseIcmpCodeIPv6 => Ok("nf.data.reverseIcmpCodeIPv6"),
        ReverseInformationElement::ReverseUdpSourcePort => Ok("nf.data.reverseUdpSourcePort"),
        ReverseInformationElement::ReverseUdpDestinationPort => {
            Ok("nf.data.reverseUdpDestinationPort")
        }
        ReverseInformationElement::ReverseTcpSourcePort => Ok("nf.data.reverseTcpSourcePort"),
        ReverseInformationElement::ReverseTcpDestinationPort => {
            Ok("nf.data.reverseTcpDestinationPort")
        }
        ReverseInformationElement::ReverseTcpSequenceNumber => {
            Ok("nf.data.reverseTcpSequenceNumber")
        }
        ReverseInformationElement::ReverseTcpAcknowledgementNumber => {
            Ok("nf.data.reverseTcpAcknowledgementNumber")
        }
        ReverseInformationElement::ReverseTcpWindowSize => Ok("nf.data.reverseTcpWindowSize"),
        ReverseInformationElement::ReverseTcpUrgentPointer => Ok("nf.data.reverseTcpUrgentPointer"),
        ReverseInformationElement::ReverseTcpHeaderLength => Ok("nf.data.reverseTcpHeaderLength"),
        ReverseInformationElement::ReverseIpHeaderLength => Ok("nf.data.reverseIpHeaderLength"),
        ReverseInformationElement::ReverseTotalLengthIPv4 => Ok("nf.data.reverseTotalLengthIPv4"),
        ReverseInformationElement::ReversePayloadLengthIPv6 => {
            Ok("nf.data.reversePayloadLengthIPv6")
        }
        ReverseInformationElement::ReverseIpTTL => Ok("nf.data.reverseIpTTL"),
        ReverseInformationElement::ReverseNextHeaderIPv6 => Ok("nf.data.reverseNextHeaderIPv6"),
        ReverseInformationElement::ReverseMplsPayloadLength => {
            Ok("nf.data.reverseMplsPayloadLength")
        }
        ReverseInformationElement::ReverseIpDiffServCodePoint => {
            Ok("nf.data.reverseIpDiffServCodePoint")
        }
        ReverseInformationElement::ReverseIpPrecedence => Ok("nf.data.reverseIpPrecedence"),
        ReverseInformationElement::ReverseFragmentFlags => Ok("nf.data.reverseFragmentFlags"),
        ReverseInformationElement::ReverseOctetDeltaSumOfSquares => {
            Ok("nf.data.reverseOctetDeltaSumOfSquares")
        }
        ReverseInformationElement::ReverseOctetTotalSumOfSquares => {
            Ok("nf.data.reverseOctetTotalSumOfSquares")
        }
        ReverseInformationElement::ReverseMplsTopLabelTTL => Ok("nf.data.reverseMplsTopLabelTTL"),
        ReverseInformationElement::ReverseMplsLabelStackLength => {
            Ok("nf.data.reverseMplsLabelStackLength")
        }
        ReverseInformationElement::ReverseMplsLabelStackDepth => {
            Ok("nf.data.reverseMplsLabelStackDepth")
        }
        ReverseInformationElement::ReverseMplsTopLabelExp => Ok("nf.data.reverseMplsTopLabelExp"),
        ReverseInformationElement::ReverseIpPayloadLength => Ok("nf.data.reverseIpPayloadLength"),
        ReverseInformationElement::ReverseUdpMessageLength => Ok("nf.data.reverseUdpMessageLength"),
        ReverseInformationElement::ReverseIsMulticast => Ok("nf.data.reverseIsMulticast"),
        ReverseInformationElement::ReverseIpv4IHL => Ok("nf.data.reverseIpv4IHL"),
        ReverseInformationElement::ReverseIpv4Options => Ok("nf.data.reverseIpv4Options"),
        ReverseInformationElement::ReverseTcpOptions => Ok("nf.data.reverseTcpOptions"),
        ReverseInformationElement::ReverseTcpSynTotalCount => Ok("nf.data.reverseTcpSynTotalCount"),
        ReverseInformationElement::ReverseTcpFinTotalCount => Ok("nf.data.reverseTcpFinTotalCount"),
        ReverseInformationElement::ReverseTcpRstTotalCount => Ok("nf.data.reverseTcpRstTotalCount"),
        ReverseInformationElement::ReverseTcpPshTotalCount => Ok("nf.data.reverseTcpPshTotalCount"),
        ReverseInformationElement::ReverseTcpAckTotalCount => Ok("nf.data.reverseTcpAckTotalCount"),
        ReverseInformationElement::ReverseTcpUrgTotalCount => Ok("nf.data.reverseTcpUrgTotalCount"),
        ReverseInformationElement::ReverseIpTotalLength => Ok("nf.data.reverseIpTotalLength"),
        ReverseInformationElement::ReversePostNATSourceIPv4Address => {
            Ok("nf.data.reversePostNATSourceIPv4Address")
        }
        ReverseInformationElement::ReversePostNATDestinationIPv4Address => {
            Ok("nf.data.reversePostNATDestinationIPv4Address")
        }
        ReverseInformationElement::ReversePostNAPTSourceTransportPort => {
            Ok("nf.data.reversePostNAPTSourceTransportPort")
        }
        ReverseInformationElement::ReversePostNAPTDestinationTransportPort => {
            Ok("nf.data.reversePostNAPTDestinationTransportPort")
        }
        ReverseInformationElement::ReverseNatOriginatingAddressRealm => {
            Ok("nf.data.reverseNatOriginatingAddressRealm")
        }
        ReverseInformationElement::ReverseNatEvent => Ok("nf.data.reverseNatEvent"),
        ReverseInformationElement::ReverseInitiatorOctets => Ok("nf.data.reverseInitiatorOctets"),
        ReverseInformationElement::ReverseResponderOctets => Ok("nf.data.reverseResponderOctets"),
        ReverseInformationElement::ReverseFirewallEvent => Ok("nf.data.reverseFirewallEvent"),
        ReverseInformationElement::ReverseIngressVRFID => Ok("nf.data.reverseIngressVRFID"),
        ReverseInformationElement::ReverseEgressVRFID => Ok("nf.data.reverseEgressVRFID"),
        ReverseInformationElement::ReverseVRFname => Ok("nf.data.reverseVRFname"),
        ReverseInformationElement::ReversePostMplsTopLabelExp => {
            Ok("nf.data.reversePostMplsTopLabelExp")
        }
        ReverseInformationElement::ReverseTcpWindowScale => Ok("nf.data.reverseTcpWindowScale"),
        ReverseInformationElement::ReverseEthernetHeaderLength => {
            Ok("nf.data.reverseEthernetHeaderLength")
        }
        ReverseInformationElement::ReverseEthernetPayloadLength => {
            Ok("nf.data.reverseEthernetPayloadLength")
        }
        ReverseInformationElement::ReverseEthernetTotalLength => {
            Ok("nf.data.reverseEthernetTotalLength")
        }
        ReverseInformationElement::ReverseDot1qVlanId => Ok("nf.data.reverseDot1qVlanId"),
        ReverseInformationElement::ReverseDot1qPriority => Ok("nf.data.reverseDot1qPriority"),
        ReverseInformationElement::ReverseDot1qCustomerVlanId => {
            Ok("nf.data.reverseDot1qCustomerVlanId")
        }
        ReverseInformationElement::ReverseDot1qCustomerPriority => {
            Ok("nf.data.reverseDot1qCustomerPriority")
        }
        ReverseInformationElement::ReverseMetroEvcId => Ok("nf.data.reverseMetroEvcId"),
        ReverseInformationElement::ReverseMetroEvcType => Ok("nf.data.reverseMetroEvcType"),
        ReverseInformationElement::ReversePseudoWireId => Ok("nf.data.reversePseudoWireId"),
        ReverseInformationElement::ReversePseudoWireType => Ok("nf.data.reversePseudoWireType"),
        ReverseInformationElement::ReversePseudoWireControlWord => {
            Ok("nf.data.reversePseudoWireControlWord")
        }
        ReverseInformationElement::ReverseIngressPhysicalInterface => {
            Ok("nf.data.reverseIngressPhysicalInterface")
        }
        ReverseInformationElement::ReverseEgressPhysicalInterface => {
            Ok("nf.data.reverseEgressPhysicalInterface")
        }
        ReverseInformationElement::ReversePostDot1qVlanId => Ok("nf.data.reversePostDot1qVlanId"),
        ReverseInformationElement::ReversePostDot1qCustomerVlanId => {
            Ok("nf.data.reversePostDot1qCustomerVlanId")
        }
        ReverseInformationElement::ReverseEthernetType => Ok("nf.data.reverseEthernetType"),
        ReverseInformationElement::ReversePostIpPrecedence => Ok("nf.data.reversePostIpPrecedence"),
        ReverseInformationElement::ReverseCollectionTimeMilliseconds => {
            Ok("nf.data.reverseCollectionTimeMilliseconds")
        }
        ReverseInformationElement::ReverseExportSctpStreamId => {
            Ok("nf.data.reverseExportSctpStreamId")
        }
        ReverseInformationElement::ReverseMaxExportSeconds => Ok("nf.data.reverseMaxExportSeconds"),
        ReverseInformationElement::ReverseMaxFlowEndSeconds => {
            Ok("nf.data.reverseMaxFlowEndSeconds")
        }
        ReverseInformationElement::ReverseMessageMD5Checksum => {
            Ok("nf.data.reverseMessageMD5Checksum")
        }
        ReverseInformationElement::ReverseMessageScope => Ok("nf.data.reverseMessageScope"),
        ReverseInformationElement::ReverseMinExportSeconds => Ok("nf.data.reverseMinExportSeconds"),
        ReverseInformationElement::ReverseMinFlowStartSeconds => {
            Ok("nf.data.reverseMinFlowStartSeconds")
        }
        ReverseInformationElement::ReverseOpaqueOctets => Ok("nf.data.reverseOpaqueOctets"),
        ReverseInformationElement::ReverseSessionScope => Ok("nf.data.reverseSessionScope"),
        ReverseInformationElement::ReverseMaxFlowEndMicroseconds => {
            Ok("nf.data.reverseMaxFlowEndMicroseconds")
        }
        ReverseInformationElement::ReverseMaxFlowEndMilliseconds => {
            Ok("nf.data.reverseMaxFlowEndMilliseconds")
        }
        ReverseInformationElement::ReverseMaxFlowEndNanoseconds => {
            Ok("nf.data.reverseMaxFlowEndNanoseconds")
        }
        ReverseInformationElement::ReverseMinFlowStartMicroseconds => {
            Ok("nf.data.reverseMinFlowStartMicroseconds")
        }
        ReverseInformationElement::ReverseMinFlowStartMilliseconds => {
            Ok("nf.data.reverseMinFlowStartMilliseconds")
        }
        ReverseInformationElement::ReverseMinFlowStartNanoseconds => {
            Ok("nf.data.reverseMinFlowStartNanoseconds")
        }
        ReverseInformationElement::ReverseCollectorCertificate => {
            Ok("nf.data.reverseCollectorCertificate")
        }
        ReverseInformationElement::ReverseExporterCertificate => {
            Ok("nf.data.reverseExporterCertificate")
        }
        ReverseInformationElement::ReverseDataRecordsReliability => {
            Ok("nf.data.reverseDataRecordsReliability")
        }
        ReverseInformationElement::ReverseObservationPointType => {
            Ok("nf.data.reverseObservationPointType")
        }
        ReverseInformationElement::ReverseNewConnectionDeltaCount => {
            Ok("nf.data.reverseNewConnectionDeltaCount")
        }
        ReverseInformationElement::ReverseConnectionSumDurationSeconds => {
            Ok("nf.data.reverseConnectionSumDurationSeconds")
        }
        ReverseInformationElement::ReverseConnectionTransactionId => {
            Ok("nf.data.reverseConnectionTransactionId")
        }
        ReverseInformationElement::ReversePostNATSourceIPv6Address => {
            Ok("nf.data.reversePostNATSourceIPv6Address")
        }
        ReverseInformationElement::ReversePostNATDestinationIPv6Address => {
            Ok("nf.data.reversePostNATDestinationIPv6Address")
        }
        ReverseInformationElement::ReverseNatPoolId => Ok("nf.data.reverseNatPoolId"),
        ReverseInformationElement::ReverseNatPoolName => Ok("nf.data.reverseNatPoolName"),
        ReverseInformationElement::ReverseAnonymizationFlags => {
            Ok("nf.data.reverseAnonymizationFlags")
        }
        ReverseInformationElement::ReverseAnonymizationTechnique => {
            Ok("nf.data.reverseAnonymizationTechnique")
        }
        ReverseInformationElement::ReverseInformationElementIndex => {
            Ok("nf.data.reverseInformationElementIndex")
        }
        ReverseInformationElement::ReverseP2pTechnology => Ok("nf.data.reverseP2pTechnology"),
        ReverseInformationElement::ReverseTunnelTechnology => Ok("nf.data.reverseTunnelTechnology"),
        ReverseInformationElement::ReverseEncryptedTechnology => {
            Ok("nf.data.reverseEncryptedTechnology")
        }
        ReverseInformationElement::ReverseBgpValidityState => Ok("nf.data.reverseBgpValidityState"),
        ReverseInformationElement::ReverseIPSecSPI => Ok("nf.data.reverseIPSecSPI"),
        ReverseInformationElement::ReverseGreKey => Ok("nf.data.reverseGreKey"),
        ReverseInformationElement::ReverseNatType => Ok("nf.data.reverseNatType"),
        ReverseInformationElement::ReverseInitiatorPackets => Ok("nf.data.reverseInitiatorPackets"),
        ReverseInformationElement::ReverseResponderPackets => Ok("nf.data.reverseResponderPackets"),
        ReverseInformationElement::ReverseObservationDomainName => {
            Ok("nf.data.reverseObservationDomainName")
        }
        ReverseInformationElement::ReverseSelectionSequenceId => {
            Ok("nf.data.reverseSelectionSequenceId")
        }
        ReverseInformationElement::ReverseSelectorId => Ok("nf.data.reverseSelectorId"),
        ReverseInformationElement::ReverseInformationElementId => {
            Ok("nf.data.reverseInformationElementId")
        }
        ReverseInformationElement::ReverseSelectorAlgorithm => {
            Ok("nf.data.reverseSelectorAlgorithm")
        }
        ReverseInformationElement::ReverseSamplingPacketInterval => {
            Ok("nf.data.reverseSamplingPacketInterval")
        }
        ReverseInformationElement::ReverseSamplingPacketSpace => {
            Ok("nf.data.reverseSamplingPacketSpace")
        }
        ReverseInformationElement::ReverseSamplingTimeInterval => {
            Ok("nf.data.reverseSamplingTimeInterval")
        }
        ReverseInformationElement::ReverseSamplingTimeSpace => {
            Ok("nf.data.reverseSamplingTimeSpace")
        }
        ReverseInformationElement::ReverseSamplingSize => Ok("nf.data.reverseSamplingSize"),
        ReverseInformationElement::ReverseSamplingPopulation => {
            Ok("nf.data.reverseSamplingPopulation")
        }
        ReverseInformationElement::ReverseSamplingProbability => {
            Ok("nf.data.reverseSamplingProbability")
        }
        ReverseInformationElement::ReverseDataLinkFrameSize => {
            Ok("nf.data.reverseDataLinkFrameSize")
        }
        ReverseInformationElement::ReverseIpHeaderPacketSection => {
            Ok("nf.data.reverseIpHeaderPacketSection")
        }
        ReverseInformationElement::ReverseIpPayloadPacketSection => {
            Ok("nf.data.reverseIpPayloadPacketSection")
        }
        ReverseInformationElement::ReverseDataLinkFrameSection => {
            Ok("nf.data.reverseDataLinkFrameSection")
        }
        ReverseInformationElement::ReverseMplsLabelStackSection => {
            Ok("nf.data.reverseMplsLabelStackSection")
        }
        ReverseInformationElement::ReverseMplsPayloadPacketSection => {
            Ok("nf.data.reverseMplsPayloadPacketSection")
        }
        ReverseInformationElement::ReverseSelectorIdTotalPktsObserved => {
            Ok("nf.data.reverseSelectorIdTotalPktsObserved")
        }
        ReverseInformationElement::ReverseSelectorIdTotalPktsSelected => {
            Ok("nf.data.reverseSelectorIdTotalPktsSelected")
        }
        ReverseInformationElement::ReverseAbsoluteError => Ok("nf.data.reverseAbsoluteError"),
        ReverseInformationElement::ReverseRelativeError => Ok("nf.data.reverseRelativeError"),
        ReverseInformationElement::ReverseObservationTimeSeconds => {
            Ok("nf.data.reverseObservationTimeSeconds")
        }
        ReverseInformationElement::ReverseObservationTimeMilliseconds => {
            Ok("nf.data.reverseObservationTimeMilliseconds")
        }
        ReverseInformationElement::ReverseObservationTimeMicroseconds => {
            Ok("nf.data.reverseObservationTimeMicroseconds")
        }
        ReverseInformationElement::ReverseObservationTimeNanoseconds => {
            Ok("nf.data.reverseObservationTimeNanoseconds")
        }
        ReverseInformationElement::ReverseDigestHashValue => Ok("nf.data.reverseDigestHashValue"),
        ReverseInformationElement::ReverseHashIPPayloadOffset => {
            Ok("nf.data.reverseHashIPPayloadOffset")
        }
        ReverseInformationElement::ReverseHashIPPayloadSize => {
            Ok("nf.data.reverseHashIPPayloadSize")
        }
        ReverseInformationElement::ReverseHashOutputRangeMin => {
            Ok("nf.data.reverseHashOutputRangeMin")
        }
        ReverseInformationElement::ReverseHashOutputRangeMax => {
            Ok("nf.data.reverseHashOutputRangeMax")
        }
        ReverseInformationElement::ReverseHashSelectedRangeMin => {
            Ok("nf.data.reverseHashSelectedRangeMin")
        }
        ReverseInformationElement::ReverseHashSelectedRangeMax => {
            Ok("nf.data.reverseHashSelectedRangeMax")
        }
        ReverseInformationElement::ReverseHashDigestOutput => Ok("nf.data.reverseHashDigestOutput"),
        ReverseInformationElement::ReverseHashInitialiserValue => {
            Ok("nf.data.reverseHashInitialiserValue")
        }
        ReverseInformationElement::ReverseSelectorName => Ok("nf.data.reverseSelectorName"),
        ReverseInformationElement::ReverseUpperCILimit => Ok("nf.data.reverseUpperCILimit"),
        ReverseInformationElement::ReverseLowerCILimit => Ok("nf.data.reverseLowerCILimit"),
        ReverseInformationElement::ReverseConfidenceLevel => Ok("nf.data.reverseConfidenceLevel"),
        ReverseInformationElement::ReverseInformationElementDataType => {
            Ok("nf.data.reverseInformationElementDataType")
        }
        ReverseInformationElement::ReverseInformationElementDescription => {
            Ok("nf.data.reverseInformationElementDescription")
        }
        ReverseInformationElement::ReverseInformationElementName => {
            Ok("nf.data.reverseInformationElementName")
        }
        ReverseInformationElement::ReverseInformationElementRangeBegin => {
            Ok("nf.data.reverseInformationElementRangeBegin")
        }
        ReverseInformationElement::ReverseInformationElementRangeEnd => {
            Ok("nf.data.reverseInformationElementRangeEnd")
        }
        ReverseInformationElement::ReverseInformationElementSemantics => {
            Ok("nf.data.reverseInformationElementSemantics")
        }
        ReverseInformationElement::ReverseInformationElementUnits => {
            Ok("nf.data.reverseInformationElementUnits")
        }
        ReverseInformationElement::ReversePrivateEnterpriseNumber => {
            Ok("nf.data.reversePrivateEnterpriseNumber")
        }
        ReverseInformationElement::ReverseVirtualStationInterfaceId => {
            Ok("nf.data.reverseVirtualStationInterfaceId")
        }
        ReverseInformationElement::ReverseVirtualStationInterfaceName => {
            Ok("nf.data.reverseVirtualStationInterfaceName")
        }
        ReverseInformationElement::ReverseVirtualStationUUID => {
            Ok("nf.data.reverseVirtualStationUUID")
        }
        ReverseInformationElement::ReverseVirtualStationName => {
            Ok("nf.data.reverseVirtualStationName")
        }
        ReverseInformationElement::ReverseLayer2SegmentId => Ok("nf.data.reverseLayer2SegmentId"),
        ReverseInformationElement::ReverseLayer2OctetDeltaCount => {
            Ok("nf.data.reverseLayer2OctetDeltaCount")
        }
        ReverseInformationElement::ReverseLayer2OctetTotalCount => {
            Ok("nf.data.reverseLayer2OctetTotalCount")
        }
        ReverseInformationElement::ReverseIngressUnicastPacketTotalCount => {
            Ok("nf.data.reverseIngressUnicastPacketTotalCount")
        }
        ReverseInformationElement::ReverseIngressMulticastPacketTotalCount => {
            Ok("nf.data.reverseIngressMulticastPacketTotalCount")
        }
        ReverseInformationElement::ReverseIngressBroadcastPacketTotalCount => {
            Ok("nf.data.reverseIngressBroadcastPacketTotalCount")
        }
        ReverseInformationElement::ReverseEgressUnicastPacketTotalCount => {
            Ok("nf.data.reverseEgressUnicastPacketTotalCount")
        }
        ReverseInformationElement::ReverseEgressBroadcastPacketTotalCount => {
            Ok("nf.data.reverseEgressBroadcastPacketTotalCount")
        }
        ReverseInformationElement::ReverseMonitoringIntervalStartMilliSeconds => {
            Ok("nf.data.reverseMonitoringIntervalStartMilliSeconds")
        }
        ReverseInformationElement::ReverseMonitoringIntervalEndMilliSeconds => {
            Ok("nf.data.reverseMonitoringIntervalEndMilliSeconds")
        }
        ReverseInformationElement::ReversePortRangeStart => Ok("nf.data.reversePortRangeStart"),
        ReverseInformationElement::ReversePortRangeEnd => Ok("nf.data.reversePortRangeEnd"),
        ReverseInformationElement::ReversePortRangeStepSize => {
            Ok("nf.data.reversePortRangeStepSize")
        }
        ReverseInformationElement::ReversePortRangeNumPorts => {
            Ok("nf.data.reversePortRangeNumPorts")
        }
        ReverseInformationElement::ReverseStaMacAddress => Ok("nf.data.reverseStaMacAddress"),
        ReverseInformationElement::ReverseStaIPv4Address => Ok("nf.data.reverseStaIPv4Address"),
        ReverseInformationElement::ReverseWtpMacAddress => Ok("nf.data.reverseWtpMacAddress"),
        ReverseInformationElement::ReverseIngressInterfaceType => {
            Ok("nf.data.reverseIngressInterfaceType")
        }
        ReverseInformationElement::ReverseEgressInterfaceType => {
            Ok("nf.data.reverseEgressInterfaceType")
        }
        ReverseInformationElement::ReverseRtpSequenceNumber => {
            Ok("nf.data.reverseRtpSequenceNumber")
        }
        ReverseInformationElement::ReverseUserName => Ok("nf.data.reverseUserName"),
        ReverseInformationElement::ReverseApplicationCategoryName => {
            Ok("nf.data.reverseApplicationCategoryName")
        }
        ReverseInformationElement::ReverseApplicationSubCategoryName => {
            Ok("nf.data.reverseApplicationSubCategoryName")
        }
        ReverseInformationElement::ReverseApplicationGroupName => {
            Ok("nf.data.reverseApplicationGroupName")
        }
        ReverseInformationElement::ReverseOriginalFlowsPresent => {
            Ok("nf.data.reverseOriginalFlowsPresent")
        }
        ReverseInformationElement::ReverseOriginalFlowsInitiated => {
            Ok("nf.data.reverseOriginalFlowsInitiated")
        }
        ReverseInformationElement::ReverseOriginalFlowsCompleted => {
            Ok("nf.data.reverseOriginalFlowsCompleted")
        }
        ReverseInformationElement::ReverseDistinctCountOfSourceIPAddress => {
            Ok("nf.data.reverseDistinctCountOfSourceIPAddress")
        }
        ReverseInformationElement::ReverseDistinctCountOfDestinationIPAddress => {
            Ok("nf.data.reverseDistinctCountOfDestinationIPAddress")
        }
        ReverseInformationElement::ReverseDistinctCountOfSourceIPv4Address => {
            Ok("nf.data.reverseDistinctCountOfSourceIPv4Address")
        }
        ReverseInformationElement::ReverseDistinctCountOfDestinationIPv4Address => {
            Ok("nf.data.reverseDistinctCountOfDestinationIPv4Address")
        }
        ReverseInformationElement::ReverseDistinctCountOfSourceIPv6Address => {
            Ok("nf.data.reverseDistinctCountOfSourceIPv6Address")
        }
        ReverseInformationElement::ReverseDistinctCountOfDestinationIPv6Address => {
            Ok("nf.data.reverseDistinctCountOfDestinationIPv6Address")
        }
        ReverseInformationElement::ReverseValueDistributionMethod => {
            Ok("nf.data.reverseValueDistributionMethod")
        }
        ReverseInformationElement::ReverseRfc3550JitterMilliseconds => {
            Ok("nf.data.reverseRfc3550JitterMilliseconds")
        }
        ReverseInformationElement::ReverseRfc3550JitterMicroseconds => {
            Ok("nf.data.reverseRfc3550JitterMicroseconds")
        }
        ReverseInformationElement::ReverseRfc3550JitterNanoseconds => {
            Ok("nf.data.reverseRfc3550JitterNanoseconds")
        }
        ReverseInformationElement::ReverseDot1qDEI => Ok("nf.data.reverseDot1qDEI"),
        ReverseInformationElement::ReverseDot1qCustomerDEI => Ok("nf.data.reverseDot1qCustomerDEI"),
        ReverseInformationElement::ReverseFlowSelectorAlgorithm => {
            Ok("nf.data.reverseFlowSelectorAlgorithm")
        }
        ReverseInformationElement::ReverseFlowSelectedOctetDeltaCount => {
            Ok("nf.data.reverseFlowSelectedOctetDeltaCount")
        }
        ReverseInformationElement::ReverseFlowSelectedPacketDeltaCount => {
            Ok("nf.data.reverseFlowSelectedPacketDeltaCount")
        }
        ReverseInformationElement::ReverseFlowSelectedFlowDeltaCount => {
            Ok("nf.data.reverseFlowSelectedFlowDeltaCount")
        }
        ReverseInformationElement::ReverseSelectorIDTotalFlowsObserved => {
            Ok("nf.data.reverseSelectorIDTotalFlowsObserved")
        }
        ReverseInformationElement::ReverseSelectorIDTotalFlowsSelected => {
            Ok("nf.data.reverseSelectorIDTotalFlowsSelected")
        }
        ReverseInformationElement::ReverseSamplingFlowInterval => {
            Ok("nf.data.reverseSamplingFlowInterval")
        }
        ReverseInformationElement::ReverseSamplingFlowSpacing => {
            Ok("nf.data.reverseSamplingFlowSpacing")
        }
        ReverseInformationElement::ReverseFlowSamplingTimeInterval => {
            Ok("nf.data.reverseFlowSamplingTimeInterval")
        }
        ReverseInformationElement::ReverseFlowSamplingTimeSpacing => {
            Ok("nf.data.reverseFlowSamplingTimeSpacing")
        }
        ReverseInformationElement::ReverseHashFlowDomain => Ok("nf.data.reverseHashFlowDomain"),
        ReverseInformationElement::ReverseTransportOctetDeltaCount => {
            Ok("nf.data.reverseTransportOctetDeltaCount")
        }
        ReverseInformationElement::ReverseTransportPacketDeltaCount => {
            Ok("nf.data.reverseTransportPacketDeltaCount")
        }
        ReverseInformationElement::ReverseOriginalExporterIPv4Address => {
            Ok("nf.data.reverseOriginalExporterIPv4Address")
        }
        ReverseInformationElement::ReverseOriginalExporterIPv6Address => {
            Ok("nf.data.reverseOriginalExporterIPv6Address")
        }
        ReverseInformationElement::ReverseOriginalObservationDomainId => {
            Ok("nf.data.reverseOriginalObservationDomainId")
        }
        ReverseInformationElement::ReverseIntermediateProcessId => {
            Ok("nf.data.reverseIntermediateProcessId")
        }
        ReverseInformationElement::ReverseIgnoredDataRecordTotalCount => {
            Ok("nf.data.reverseIgnoredDataRecordTotalCount")
        }
        ReverseInformationElement::ReverseDataLinkFrameType => {
            Ok("nf.data.reverseDataLinkFrameType")
        }
        ReverseInformationElement::ReverseSectionOffset => Ok("nf.data.reverseSectionOffset"),
        ReverseInformationElement::ReverseSectionExportedOctets => {
            Ok("nf.data.reverseSectionExportedOctets")
        }
        ReverseInformationElement::ReverseDot1qServiceInstanceTag => {
            Ok("nf.data.reverseDot1qServiceInstanceTag")
        }
        ReverseInformationElement::ReverseDot1qServiceInstanceId => {
            Ok("nf.data.reverseDot1qServiceInstanceId")
        }
        ReverseInformationElement::ReverseDot1qServiceInstancePriority => {
            Ok("nf.data.reverseDot1qServiceInstancePriority")
        }
        ReverseInformationElement::ReverseDot1qCustomerSourceMacAddress => {
            Ok("nf.data.reverseDot1qCustomerSourceMacAddress")
        }
        ReverseInformationElement::ReverseDot1qCustomerDestinationMacAddress => {
            Ok("nf.data.reverseDot1qCustomerDestinationMacAddress")
        }
        ReverseInformationElement::ReversePostLayer2OctetDeltaCount => {
            Ok("nf.data.reversePostLayer2OctetDeltaCount")
        }
        ReverseInformationElement::ReversePostMCastLayer2OctetDeltaCount => {
            Ok("nf.data.reversePostMCastLayer2OctetDeltaCount")
        }
        ReverseInformationElement::ReversePostLayer2OctetTotalCount => {
            Ok("nf.data.reversePostLayer2OctetTotalCount")
        }
        ReverseInformationElement::ReversePostMCastLayer2OctetTotalCount => {
            Ok("nf.data.reversePostMCastLayer2OctetTotalCount")
        }
        ReverseInformationElement::ReverseMinimumLayer2TotalLength => {
            Ok("nf.data.reverseMinimumLayer2TotalLength")
        }
        ReverseInformationElement::ReverseMaximumLayer2TotalLength => {
            Ok("nf.data.reverseMaximumLayer2TotalLength")
        }
        ReverseInformationElement::ReverseDroppedLayer2OctetDeltaCount => {
            Ok("nf.data.reverseDroppedLayer2OctetDeltaCount")
        }
        ReverseInformationElement::ReverseDroppedLayer2OctetTotalCount => {
            Ok("nf.data.reverseDroppedLayer2OctetTotalCount")
        }
        ReverseInformationElement::ReverseIgnoredLayer2OctetTotalCount => {
            Ok("nf.data.reverseIgnoredLayer2OctetTotalCount")
        }
        ReverseInformationElement::ReverseNotSentLayer2OctetTotalCount => {
            Ok("nf.data.reverseNotSentLayer2OctetTotalCount")
        }
        ReverseInformationElement::ReverseLayer2OctetDeltaSumOfSquares => {
            Ok("nf.data.reverseLayer2OctetDeltaSumOfSquares")
        }
        ReverseInformationElement::ReverseLayer2OctetTotalSumOfSquares => {
            Ok("nf.data.reverseLayer2OctetTotalSumOfSquares")
        }
        ReverseInformationElement::ReverseLayer2FrameDeltaCount => {
            Ok("nf.data.reverseLayer2FrameDeltaCount")
        }
        ReverseInformationElement::ReverseLayer2FrameTotalCount => {
            Ok("nf.data.reverseLayer2FrameTotalCount")
        }
        ReverseInformationElement::ReversePseudoWireDestinationIPv4Address => {
            Ok("nf.data.reversePseudoWireDestinationIPv4Address")
        }
        ReverseInformationElement::ReverseIgnoredLayer2FrameTotalCount => {
            Ok("nf.data.reverseIgnoredLayer2FrameTotalCount")
        }
        ReverseInformationElement::Unknown(field_num) => Err(*field_num),
    }
}

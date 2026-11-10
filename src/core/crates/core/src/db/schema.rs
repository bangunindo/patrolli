// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use sea_query::{Expr, Iden, Value};
use serde_repr::{Deserialize_repr, Serialize_repr};
use std::fmt::{Display, Formatter};
use std::str::FromStr;
use utoipa::ToSchema;

#[derive(Iden)]
pub enum ClusterNodes {
    Table,
    NodeId,
    Name,
    OpStatus,
    IsLeader,
    LastHeartbeat,
    Metadata,
}

#[derive(Iden)]
pub enum ClusterLogs {
    Table,
    Ts,
    /// Insertion order, which `Ts` cannot give within one transaction.
    Seq,
    NodeId,
    Resource,
    ResourceId,
    Op,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClusterLogResource {
    Host,
    HostEndpoint,
    Item,
    Listener,
    ListenerItem,
    ListenerEndpoint,
    LookupTable,
    Processor,
    Sink,
    ClusterNode,
}

impl ClusterLogResource {
    pub fn as_str(&self) -> &'static str {
        match self {
            ClusterLogResource::Host => "host",
            ClusterLogResource::HostEndpoint => "host_endpoint",
            ClusterLogResource::Item => "item",
            ClusterLogResource::Listener => "listener",
            ClusterLogResource::ListenerItem => "listener_item",
            ClusterLogResource::ListenerEndpoint => "listener_endpoint",
            ClusterLogResource::LookupTable => "lookup_table",
            ClusterLogResource::Processor => "processor",
            ClusterLogResource::Sink => "sink",
            ClusterLogResource::ClusterNode => "cluster_node",
        }
    }
}

impl Display for ClusterLogResource {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl FromStr for ClusterLogResource {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> anyhow::Result<Self, Self::Err> {
        match s {
            "host" => Ok(ClusterLogResource::Host),
            "host_endpoint" => Ok(ClusterLogResource::HostEndpoint),
            "item" => Ok(ClusterLogResource::Item),
            "listener" => Ok(ClusterLogResource::Listener),
            "listener_item" => Ok(ClusterLogResource::ListenerItem),
            "listener_endpoint" => Ok(ClusterLogResource::ListenerEndpoint),
            "lookup_table" => Ok(ClusterLogResource::LookupTable),
            "processor" => Ok(ClusterLogResource::Processor),
            "sink" => Ok(ClusterLogResource::Sink),
            "cluster_node" => Ok(ClusterLogResource::ClusterNode),
            _ => Err(anyhow::anyhow!("invalid ClusterLogResources value")),
        }
    }
}

#[derive(Iden)]
pub enum Hosts {
    Table,
    HostId,
    Name,
    AdmStatus,
    Labels,
    LastUpdated,
    MonitoredBy,
    Timezone,
}

#[derive(Iden)]
pub enum Templates {
    Table,
    TemplateId,
    Name,
    Labels,
}

#[derive(Iden)]
pub enum TemplateLinks {
    Table,
    TemplateLinkId,
    TemplateId,
    LinkToTemplateId,
}

#[derive(Iden)]
pub enum HostTemplates {
    Table,
    HostTemplateId,
    HostId,
    TemplateId,
}

#[derive(Iden)]
pub enum Listeners {
    Table,
    ListenerId,
    Name,
    Type,
    AdmStatus,
    Labels,
    LastUpdated,
    WorkerCount,
    BatchSize,
    BatchTimeoutMs,
    Config,
    DisableClusterForwarding,
}

#[derive(Iden)]
pub enum HostEndpoints {
    Table,
    EndpointId,
    HostId,
    Name,
    Type,
    AdmStatus,
    LastUpdated,
    Config,
}

#[derive(Iden)]
pub enum Processors {
    Table,
    ProcessorId,
    Name,
    Type,
    AdmStatus,
    LastUpdated,
    Config,
}

#[derive(Iden)]
pub enum ProcessorInputs {
    Table,
    ProcessorId,
    InputId,
}

#[derive(Iden)]
pub enum Items {
    Table,
    ItemId,
    HostTemplateId,
    TemplateItemId,
    ListenerId,
    EndpointId,
    Type,
    Flags,
    AdmStatus,
    Name,
    Schedule,
    TimeoutMs,
    LastUpdated,
    Config,
    Labels,
}

#[derive(Iden)]
pub enum ItemProcessors {
    Table,
    ItemId,
    ProcessorId,
}

#[derive(Iden)]
pub enum Sinks {
    Table,
    SinkId,
    Name,
    Type,
    AdmStatus,
    Labels,
    LastUpdated,
    WorkerCount,
    BatchSize,
    BatchTimeoutMs,
    Channel,
    Retry,
    Config,
}

#[derive(Iden)]
pub enum SinkInputs {
    Table,
    SinkId,
    InputId,
}

#[derive(Iden)]
pub enum Configs {
    Table,
    Key,
    Val,
    LastUpdated,
}

/// `Type`/`Source` and `KeyType`/`Key` are tag-and-body pairs: the enum's
/// discriminant lives in the `TEXT` column and its variant payload in the
/// `JSONB` one, recombined on read by [`crate::db::query::LookupTableColumn`].
#[derive(Iden)]
pub enum LookupTables {
    Table,
    LookupTableId,
    Name,
    Type,
    AdmStatus,
    Labels,
    LastUpdated,
    Source,
    KeyType,
    Key,
}

#[derive(Iden)]
pub enum Users {
    Table,
    UserId,
    Username,
    PasswordHash,
    DisplayName,
    Email,
    AdmStatus,
    AllowSso,
    OidcSubject,
    LastUpdated,
}

#[derive(Iden)]
pub enum RefreshTokens {
    Table,
    TokenHash,
    UserId,
    ExpiresAt,
}

// the column name repeats the table name
#[allow(clippy::enum_variant_names)]
#[derive(Iden)]
pub enum Pipelines {
    Table,
    PipelineId,
    Name,
    Visibility,
    OwnerUserId,
    AdmStatus,
    CreatedAt,
    LastUpdated,
}

#[derive(Iden)]
pub enum PipelineHosts {
    Table,
    PipelineId,
    HostId,
}

#[derive(Iden)]
pub enum PipelineSinks {
    Table,
    PipelineId,
    SinkId,
}

#[derive(Iden)]
pub enum PipelineCanvases {
    Table,
    CanvasId,
    PipelineId,
    LastUpdated,
}

#[derive(Iden)]
pub enum PipelineNodes {
    Table,
    NodeId,
    CanvasId,
    Kind,
    PositionX,
    PositionY,
    LastUpdated,
}

#[derive(Iden)]
pub enum PipelineNodeSources {
    Table,
    NodeId,
    SourceProcessorId,
    GateProcessorId,
}

#[derive(Iden)]
pub enum PipelineNodeSourceItems {
    Table,
    NodeId,
    ItemId,
}

#[derive(Iden)]
pub enum PipelineNodeProcessorGroups {
    Table,
    NodeId,
}

#[derive(Iden)]
pub enum PipelineNodeProcessors {
    Table,
    ProcessorId,
    NodeId,
    Position,
}

#[derive(Iden)]
pub enum PipelineNodeSinks {
    Table,
    NodeId,
    SinkId,
}

#[derive(Iden)]
pub enum PipelineNodeRelations {
    Table,
    CanvasId,
    FromNodeId,
    ToNodeId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize_repr, Deserialize_repr, ToSchema)]
#[repr(i16)]
pub enum Visibility {
    Private = 0,
    Public = 1,
}

impl TryFrom<i16> for Visibility {
    type Error = anyhow::Error;

    fn try_from(value: i16) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Visibility::Private),
            1 => Ok(Visibility::Public),
            _ => Err(anyhow::anyhow!("invalid Visibility value")),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize_repr, Deserialize_repr, ToSchema)]
#[repr(i16)]
pub enum AdmStatus {
    Disabled = 0,
    Enabled = 1,
}

pub fn default_adm_status() -> AdmStatus {
    AdmStatus::Enabled
}

impl TryFrom<i16> for AdmStatus {
    type Error = anyhow::Error;

    fn try_from(value: i16) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(AdmStatus::Disabled),
            1 => Ok(AdmStatus::Enabled),
            _ => Err(anyhow::anyhow!("invalid AdmStatus value")),
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub enum ConfigKeys {
    Monitor,
    Cluster,
}

impl From<ConfigKeys> for &str {
    fn from(value: ConfigKeys) -> Self {
        match value {
            ConfigKeys::Monitor => "monitor",
            ConfigKeys::Cluster => "cluster",
        }
    }
}

impl Display for ConfigKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name: &str = (*self).into();
        write!(f, "{}", name)
    }
}

impl From<ConfigKeys> for Expr {
    fn from(value: ConfigKeys) -> Self {
        Expr::Value(Value::String(Some(value.to_string())))
    }
}

#[repr(i16)]
pub enum ClusterOpStatus {
    Inactive = 0,
    Active = 1,
    Initializing = 2,
}

#[repr(i16)]
pub enum ClusterLogsOp {
    Upsert = 1,
    Delete = 2,
}

impl TryFrom<i16> for ClusterLogsOp {
    type Error = anyhow::Error;

    fn try_from(value: i16) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(ClusterLogsOp::Upsert),
            2 => Ok(ClusterLogsOp::Delete),
            _ => Err(anyhow::anyhow!("invalid ClusterLogsOp value")),
        }
    }
}

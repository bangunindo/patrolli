// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use super::schema::*;
use sea_query::*;
use sea_query_postgres::{PostgresBinder, PostgresValues};
use std::fmt::{Display, Formatter};
use uuid::Uuid;

#[derive(Iden)]
#[iden = "least"]
struct Least;

#[derive(Iden)]
#[iden = "array_length"]
struct ArrayLength;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum SinkColumn {
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
    ConfigEmbed,
    Inputs,
    InputCount,
}

impl SinkColumn {
    pub fn as_str(&self) -> &'static str {
        match self {
            SinkColumn::SinkId => "sink_id",
            SinkColumn::Name => "name",
            SinkColumn::Type => "type",
            SinkColumn::AdmStatus => "adm_status",
            SinkColumn::Labels => "labels",
            SinkColumn::LastUpdated => "last_updated",
            SinkColumn::WorkerCount => "worker_count",
            SinkColumn::BatchSize => "batch_size",
            SinkColumn::BatchTimeoutMs => "batch_timeout_ms",
            SinkColumn::Channel => "channel",
            SinkColumn::Retry => "retry",
            SinkColumn::Config => "config",
            SinkColumn::ConfigEmbed => "config_embed",
            SinkColumn::Inputs => "inputs",
            SinkColumn::InputCount => "input_count",
        }
    }
}

impl Display for SinkColumn {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl From<SinkColumn> for Expr {
    fn from(value: SinkColumn) -> Self {
        match value {
            SinkColumn::SinkId => Expr::col((Sinks::Table, Sinks::SinkId)),
            SinkColumn::Name => Expr::col((Sinks::Table, Sinks::Name)),
            SinkColumn::Type => Expr::col((Sinks::Table, Sinks::Type)),
            SinkColumn::AdmStatus => Expr::col((Sinks::Table, Sinks::AdmStatus)),
            SinkColumn::Labels => Expr::col((Sinks::Table, Sinks::Labels)),
            SinkColumn::LastUpdated => Expr::col((Sinks::Table, Sinks::LastUpdated)),
            SinkColumn::WorkerCount => Expr::col((Sinks::Table, Sinks::WorkerCount)),
            SinkColumn::BatchSize => Expr::col((Sinks::Table, Sinks::BatchSize)),
            SinkColumn::BatchTimeoutMs => Expr::col((Sinks::Table, Sinks::BatchTimeoutMs)),
            SinkColumn::Channel => Expr::col((Sinks::Table, Sinks::Channel)),
            SinkColumn::Retry => Expr::col((Sinks::Table, Sinks::Retry)),
            SinkColumn::Config => Expr::col((Sinks::Table, Sinks::Config)),
            SinkColumn::ConfigEmbed => PgFunc::json_build_object(vec![
                (
                    Expr::val("type").cast_as("text"),
                    Expr::col((Sinks::Table, Sinks::Type)),
                ),
                (
                    Expr::val("config").cast_as("text"),
                    Expr::col((Sinks::Table, Sinks::Config)),
                ),
            ])
            .into(),
            SinkColumn::Inputs => Expr::cust(
                "COALESCE(array_agg(sink_inputs.input_id) FILTER (WHERE sink_inputs.input_id IS NOT NULL), '{}')",
            ),
            SinkColumn::InputCount => Expr::cust(
                "COALESCE(array_length(array_agg(sink_inputs.input_id) FILTER (WHERE sink_inputs.input_id IS NOT NULL), 1), 0)",
            ),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SinkParams {
    columns: Vec<SinkColumn>,
    id_subquery_filter: Option<Expr>,
    only_enabled: bool,
}

impl Default for SinkParams {
    fn default() -> Self {
        Self::new()
    }
}

impl SinkParams {
    pub fn new() -> Self {
        Self {
            columns: vec![],
            id_subquery_filter: None,
            only_enabled: false,
        }
    }

    pub fn with_columns(&mut self, columns: Vec<SinkColumn>) -> &mut Self {
        self.columns = columns;
        self
    }

    pub fn with_id_subquery_filter(&mut self, filter: Option<Expr>) -> &mut Self {
        self.id_subquery_filter = filter;
        self
    }

    pub fn with_only_enabled(&mut self, only_enabled: bool) -> &mut Self {
        self.only_enabled = only_enabled;
        self
    }
}

pub fn sinks(params: SinkParams) -> SelectStatement {
    let mut query = Query::select();
    query.from(Sinks::Table);
    for column in &params.columns {
        query.expr_as(*column, column.as_str());
    }
    if let Some(filter) = params.id_subquery_filter {
        query.and_where(SinkColumn::SinkId.binary(types::BinOper::In, filter));
    }
    if params.only_enabled {
        query.and_where(SinkColumn::AdmStatus.eq(AdmStatus::Enabled as i16));
    }
    if params.columns.contains(&SinkColumn::Inputs)
        || params.columns.contains(&SinkColumn::InputCount)
    {
        query
            .join(
                JoinType::LeftJoin,
                SinkInputs::Table,
                Expr::col((SinkInputs::Table, SinkInputs::SinkId))
                    .eq(Expr::col((Sinks::Table, Sinks::SinkId))),
            )
            .add_group_by([SinkColumn::SinkId.into()]);
    };
    query
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ProcessorColumn {
    ProcessorId,
    Name,
    Type,
    AdmStatus,
    LastUpdated,
    Config,
    ConfigEmbed,
    Inputs,
    InputItems,
}

impl ProcessorColumn {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProcessorColumn::ProcessorId => "processor_id",
            ProcessorColumn::Name => "name",
            ProcessorColumn::Type => "type",
            ProcessorColumn::AdmStatus => "adm_status",
            ProcessorColumn::LastUpdated => "last_updated",
            ProcessorColumn::Config => "config",
            ProcessorColumn::ConfigEmbed => "config_embed",
            ProcessorColumn::Inputs => "inputs",
            ProcessorColumn::InputItems => "input_items",
        }
    }
}

impl Display for ProcessorColumn {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl From<ProcessorColumn> for Expr {
    fn from(value: ProcessorColumn) -> Self {
        match value {
            ProcessorColumn::ProcessorId => Expr::col((Processors::Table, Processors::ProcessorId)),
            ProcessorColumn::Name => Expr::col((Processors::Table, Processors::Name)),
            ProcessorColumn::Type => Expr::col((Processors::Table, Processors::Type)),
            ProcessorColumn::AdmStatus => Expr::col((Processors::Table, Processors::AdmStatus)),
            ProcessorColumn::LastUpdated => Expr::col((Processors::Table, Processors::LastUpdated)),
            ProcessorColumn::Config => Expr::col((Processors::Table, Processors::Config)),
            ProcessorColumn::ConfigEmbed => PgFunc::json_build_object(vec![
                (
                    Expr::val("type").cast_as("text"),
                    Expr::col((Processors::Table, Processors::Type)),
                ),
                (
                    Expr::val("config").cast_as("text"),
                    Expr::col((Processors::Table, Processors::Config)),
                ),
            ])
            .into(),
            ProcessorColumn::Inputs => Expr::cust(
                "COALESCE(array_agg(processor_inputs.input_id) FILTER (WHERE processor_inputs.input_id IS NOT NULL), '{}')",
            ),
            ProcessorColumn::InputItems => Expr::cust(
                "COALESCE(array_agg(item_processors.item_id) FILTER (WHERE item_processors.item_id IS NOT NULL), '{}')",
            ),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProcessorParams {
    columns: Vec<ProcessorColumn>,
    id_subquery_filter: Option<Expr>,
    disabled_to_null: bool,
}

impl Default for ProcessorParams {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessorParams {
    pub fn new() -> Self {
        Self {
            columns: vec![],
            id_subquery_filter: None,
            disabled_to_null: false,
        }
    }

    pub fn with_columns(&mut self, columns: Vec<ProcessorColumn>) -> &mut Self {
        self.columns = columns;
        self
    }

    pub fn with_id_subquery_filter(&mut self, filter: Option<Expr>) -> &mut Self {
        self.id_subquery_filter = filter;
        self
    }

    pub fn with_disabled_to_null(&mut self, disabled_to_null: bool) -> &mut Self {
        self.disabled_to_null = disabled_to_null;
        self
    }
}

pub fn processors(params: ProcessorParams) -> SelectStatement {
    let mut query = Query::select();
    query.from(Processors::Table);
    for column in &params.columns {
        if params.disabled_to_null && *column == ProcessorColumn::Config {
            query.expr_as(
                Expr::case(
                    ProcessorColumn::AdmStatus.eq(AdmStatus::Disabled as i16),
                    Expr::cust("'null'::jsonb"),
                )
                .finally(ProcessorColumn::Config),
                column.as_str(),
            );
        } else if params.disabled_to_null && *column == ProcessorColumn::Type {
            query.expr_as(
                Expr::case(
                    ProcessorColumn::AdmStatus.eq(AdmStatus::Disabled as i16),
                    Expr::val("null"),
                )
                .finally(ProcessorColumn::Type),
                column.as_str(),
            );
        } else if params.disabled_to_null && *column == ProcessorColumn::ConfigEmbed {
            query.expr_as(
                Expr::case(
                    ProcessorColumn::AdmStatus.eq(AdmStatus::Disabled as i16),
                    PgFunc::json_build_object::<Expr>(vec![
                        (
                            Expr::val("type").cast_as("text"),
                            Expr::val("null").cast_as("text"),
                        ),
                        (
                            Expr::val("config").cast_as("text"),
                            Expr::cust("'null'::json"),
                        ),
                    ]),
                )
                .finally(ProcessorColumn::ConfigEmbed),
                column.as_str(),
            );
        } else {
            query.expr_as(*column, column.as_str());
        }
    }
    if let Some(filter) = params.id_subquery_filter {
        query.and_where(ProcessorColumn::ProcessorId.binary(types::BinOper::In, filter));
    }
    if params.columns.contains(&ProcessorColumn::Inputs) {
        query
            .join(
                JoinType::LeftJoin,
                ProcessorInputs::Table,
                Expr::col((ProcessorInputs::Table, ProcessorInputs::ProcessorId))
                    .eq(Expr::col((Processors::Table, Processors::ProcessorId))),
            )
            .add_group_by([ProcessorColumn::ProcessorId.into()]);
    };
    if params.columns.contains(&ProcessorColumn::InputItems) {
        query
            .join(
                JoinType::LeftJoin,
                ItemProcessors::Table,
                Expr::col((ItemProcessors::Table, ItemProcessors::ProcessorId))
                    .eq(Expr::col((Processors::Table, Processors::ProcessorId))),
            )
            .add_group_by([ProcessorColumn::ProcessorId.into()]);
    };
    query
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ListenerColumn {
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
    ConfigEmbed,
    DisableClusterForwarding,
}

impl ListenerColumn {
    pub fn as_str(&self) -> &'static str {
        match self {
            ListenerColumn::ListenerId => "listener_id",
            ListenerColumn::Name => "name",
            ListenerColumn::Type => "type",
            ListenerColumn::AdmStatus => "adm_status",
            ListenerColumn::Labels => "labels",
            ListenerColumn::LastUpdated => "last_updated",
            ListenerColumn::WorkerCount => "worker_count",
            ListenerColumn::BatchSize => "batch_size",
            ListenerColumn::BatchTimeoutMs => "batch_timeout_ms",
            ListenerColumn::Config => "config",
            ListenerColumn::ConfigEmbed => "config_embed",
            ListenerColumn::DisableClusterForwarding => "disable_cluster_forwarding",
        }
    }
}

impl Display for ListenerColumn {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl From<ListenerColumn> for Expr {
    fn from(value: ListenerColumn) -> Self {
        match value {
            ListenerColumn::ListenerId => Expr::col((Listeners::Table, Listeners::ListenerId)),
            ListenerColumn::Name => Expr::col((Listeners::Table, Listeners::Name)),
            ListenerColumn::Type => Expr::col((Listeners::Table, Listeners::Type)),
            ListenerColumn::AdmStatus => Expr::col((Listeners::Table, Listeners::AdmStatus)),
            ListenerColumn::Labels => Expr::col((Listeners::Table, Listeners::Labels)),
            ListenerColumn::LastUpdated => Expr::col((Listeners::Table, Listeners::LastUpdated)),
            ListenerColumn::WorkerCount => Expr::col((Listeners::Table, Listeners::WorkerCount)),
            ListenerColumn::BatchSize => Expr::col((Listeners::Table, Listeners::BatchSize)),
            ListenerColumn::BatchTimeoutMs => {
                Expr::col((Listeners::Table, Listeners::BatchTimeoutMs))
            }
            ListenerColumn::Config => Expr::col((Listeners::Table, Listeners::Config)),
            ListenerColumn::ConfigEmbed => PgFunc::json_build_object(vec![
                (
                    Expr::val("type").cast_as("text"),
                    Expr::col((Listeners::Table, Listeners::Type)),
                ),
                (
                    Expr::val("config").cast_as("text"),
                    Expr::col((Listeners::Table, Listeners::Config)),
                ),
            ])
            .into(),
            ListenerColumn::DisableClusterForwarding => {
                Expr::col((Listeners::Table, Listeners::DisableClusterForwarding))
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct ListenerParams {
    columns: Vec<ListenerColumn>,
    id_subquery_filter: Option<Expr>,
    only_enabled: bool,
}

impl Default for ListenerParams {
    fn default() -> Self {
        Self::new()
    }
}

impl ListenerParams {
    pub fn new() -> Self {
        Self {
            columns: vec![],
            id_subquery_filter: None,
            only_enabled: false,
        }
    }

    pub fn with_columns(&mut self, columns: Vec<ListenerColumn>) -> &mut Self {
        self.columns = columns;
        self
    }

    pub fn with_id_subquery_filter(&mut self, filter: Option<Expr>) -> &mut Self {
        self.id_subquery_filter = filter;
        self
    }

    pub fn with_only_enabled(&mut self, only_enabled: bool) -> &mut Self {
        self.only_enabled = only_enabled;
        self
    }
}

pub fn listeners(params: ListenerParams) -> SelectStatement {
    let mut query = Query::select();
    query.from(Listeners::Table);
    for column in &params.columns {
        query.expr_as(*column, column.as_str());
    }
    if let Some(filter) = params.id_subquery_filter {
        query.and_where(ListenerColumn::ListenerId.binary(BinOper::In, filter));
    }
    if params.only_enabled {
        query.and_where(ListenerColumn::AdmStatus.eq(AdmStatus::Enabled as i16));
    }
    query
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum LookupTableColumn {
    LookupTableId,
    Name,
    Type,
    AdmStatus,
    Labels,
    LastUpdated,
    Source,
    SourceEmbed,
    KeyType,
    Key,
    KeyEmbed,
}

impl LookupTableColumn {
    pub fn as_str(&self) -> &'static str {
        match self {
            LookupTableColumn::LookupTableId => "lookup_table_id",
            LookupTableColumn::Name => "name",
            LookupTableColumn::Type => "type",
            LookupTableColumn::AdmStatus => "adm_status",
            LookupTableColumn::Labels => "labels",
            LookupTableColumn::LastUpdated => "last_updated",
            LookupTableColumn::Source => "source",
            LookupTableColumn::SourceEmbed => "source_embed",
            LookupTableColumn::KeyType => "key_type",
            LookupTableColumn::Key => "key",
            LookupTableColumn::KeyEmbed => "key_embed",
        }
    }
}

impl Display for LookupTableColumn {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl From<LookupTableColumn> for Expr {
    fn from(value: LookupTableColumn) -> Self {
        match value {
            LookupTableColumn::LookupTableId => {
                Expr::col((LookupTables::Table, LookupTables::LookupTableId))
            }
            LookupTableColumn::Name => Expr::col((LookupTables::Table, LookupTables::Name)),
            LookupTableColumn::Type => Expr::col((LookupTables::Table, LookupTables::Type)),
            LookupTableColumn::AdmStatus => {
                Expr::col((LookupTables::Table, LookupTables::AdmStatus))
            }
            LookupTableColumn::Labels => Expr::col((LookupTables::Table, LookupTables::Labels)),
            LookupTableColumn::LastUpdated => {
                Expr::col((LookupTables::Table, LookupTables::LastUpdated))
            }
            LookupTableColumn::Source => Expr::col((LookupTables::Table, LookupTables::Source)),
            // `Source` and `Key` are internally tagged, so tag and body
            // recombine with a jsonb merge rather than the adjacent-tag
            // `json_build_object` of `SinkColumn::ConfigEmbed`. Neither body
            // carries a `type` key, so the merge shadows nothing.
            LookupTableColumn::SourceEmbed => Expr::cust(
                r#"jsonb_build_object('type', "lookup_tables"."type") || "lookup_tables"."source""#,
            ),
            LookupTableColumn::KeyType => Expr::col((LookupTables::Table, LookupTables::KeyType)),
            LookupTableColumn::Key => Expr::col((LookupTables::Table, LookupTables::Key)),
            LookupTableColumn::KeyEmbed => Expr::cust(
                r#"jsonb_build_object('type', "lookup_tables"."key_type") || "lookup_tables"."key""#,
            ),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LookupTableParams {
    columns: Vec<LookupTableColumn>,
    id_subquery_filter: Option<Expr>,
    only_enabled: bool,
}

impl Default for LookupTableParams {
    fn default() -> Self {
        Self::new()
    }
}

impl LookupTableParams {
    pub fn new() -> Self {
        Self {
            columns: vec![],
            id_subquery_filter: None,
            only_enabled: false,
        }
    }

    pub fn with_columns(&mut self, columns: Vec<LookupTableColumn>) -> &mut Self {
        self.columns = columns;
        self
    }

    pub fn with_id_subquery_filter(&mut self, filter: Option<Expr>) -> &mut Self {
        self.id_subquery_filter = filter;
        self
    }

    pub fn with_only_enabled(&mut self, only_enabled: bool) -> &mut Self {
        self.only_enabled = only_enabled;
        self
    }
}

pub fn lookup_tables(params: LookupTableParams) -> SelectStatement {
    let mut query = Query::select();
    query.from(LookupTables::Table);
    for column in &params.columns {
        query.expr_as(*column, column.as_str());
    }
    if let Some(filter) = params.id_subquery_filter {
        query.and_where(LookupTableColumn::LookupTableId.binary(BinOper::In, filter));
    }
    if params.only_enabled {
        query.and_where(LookupTableColumn::AdmStatus.eq(AdmStatus::Enabled as i16));
    }
    query
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum HostColumn {
    HostId,
    Name,
    AdmStatus,
    Labels,
    LastUpdated,
    MonitoredBy,
    Timezone,
}

impl HostColumn {
    pub fn as_str(&self) -> &'static str {
        match self {
            HostColumn::HostId => "host_id",
            HostColumn::Name => "name",
            HostColumn::AdmStatus => "adm_status",
            HostColumn::Labels => "labels",
            HostColumn::LastUpdated => "last_updated",
            HostColumn::MonitoredBy => "monitored_by",
            HostColumn::Timezone => "timezone",
        }
    }
}

impl Display for HostColumn {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl From<HostColumn> for Expr {
    fn from(value: HostColumn) -> Self {
        match value {
            HostColumn::HostId => Expr::col((Hosts::Table, Hosts::HostId)),
            HostColumn::Name => Expr::col((Hosts::Table, Hosts::Name)),
            HostColumn::AdmStatus => Expr::col((Hosts::Table, Hosts::AdmStatus)),
            HostColumn::Labels => Expr::col((Hosts::Table, Hosts::Labels)),
            HostColumn::LastUpdated => Expr::col((Hosts::Table, Hosts::LastUpdated)),
            HostColumn::MonitoredBy => Expr::col((Hosts::Table, Hosts::MonitoredBy)),
            HostColumn::Timezone => Expr::col((Hosts::Table, Hosts::Timezone)),
        }
    }
}

#[derive(Debug, Clone)]
pub struct HostParams {
    columns: Vec<HostColumn>,
    id_subquery_filter: Option<Expr>,
    only_enabled: bool,
    monitored_by: Option<Uuid>,
}

impl Default for HostParams {
    fn default() -> Self {
        Self::new()
    }
}

impl HostParams {
    pub fn new() -> Self {
        Self {
            columns: vec![],
            id_subquery_filter: None,
            only_enabled: false,
            monitored_by: None,
        }
    }

    pub fn with_columns(&mut self, columns: Vec<HostColumn>) -> &mut Self {
        self.columns = columns;
        self
    }

    pub fn with_id_subquery_filter(&mut self, filter: Option<Expr>) -> &mut Self {
        self.id_subquery_filter = filter;
        self
    }

    pub fn with_only_enabled(&mut self, only_enabled: bool) -> &mut Self {
        self.only_enabled = only_enabled;
        self
    }

    pub fn with_monitored_by(&mut self, monitored_by: Option<Uuid>) -> &mut Self {
        self.monitored_by = monitored_by;
        self
    }
}

pub fn hosts(params: HostParams) -> SelectStatement {
    let mut query = Query::select();
    query.from(Hosts::Table);
    for column in &params.columns {
        query.expr_as(*column, column.as_str());
    }
    if let Some(filter) = params.id_subquery_filter {
        query.and_where(HostColumn::HostId.binary(types::BinOper::In, filter));
    }
    if let Some(monitored_by) = params.monitored_by {
        query.and_where(HostColumn::MonitoredBy.eq(monitored_by));
    }
    if params.only_enabled {
        query.and_where(HostColumn::AdmStatus.eq(AdmStatus::Enabled as i16));
    }
    query
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum EndpointColumn {
    EndpointId,
    HostId,
    Name,
    Type,
    AdmStatus,
    LastUpdated,
    Config,
    ConfigEmbed,
    ResolvedAdmStatus,
    MonitoredBy,
}

impl EndpointColumn {
    pub fn as_str(&self) -> &'static str {
        match self {
            EndpointColumn::EndpointId => "endpoint_id",
            EndpointColumn::HostId => "host_id",
            EndpointColumn::Name => "name",
            EndpointColumn::Type => "type",
            EndpointColumn::AdmStatus => "adm_status",
            EndpointColumn::LastUpdated => "last_updated",
            EndpointColumn::Config => "config",
            EndpointColumn::ConfigEmbed => "config_embed",
            EndpointColumn::ResolvedAdmStatus => "resolved_adm_status",
            EndpointColumn::MonitoredBy => "monitored_by",
        }
    }
}

impl Display for EndpointColumn {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl From<EndpointColumn> for Expr {
    fn from(value: EndpointColumn) -> Self {
        match value {
            EndpointColumn::EndpointId => {
                Expr::col((HostEndpoints::Table, HostEndpoints::EndpointId))
            }
            EndpointColumn::HostId => Expr::col((HostEndpoints::Table, HostEndpoints::HostId)),
            EndpointColumn::Name => Expr::col((HostEndpoints::Table, HostEndpoints::Name)),
            EndpointColumn::Type => Expr::col((HostEndpoints::Table, HostEndpoints::Type)),
            EndpointColumn::AdmStatus => {
                Expr::col((HostEndpoints::Table, HostEndpoints::AdmStatus))
            }
            EndpointColumn::LastUpdated => {
                Expr::col((HostEndpoints::Table, HostEndpoints::LastUpdated))
            }
            EndpointColumn::Config => Expr::col((HostEndpoints::Table, HostEndpoints::Config)),
            EndpointColumn::ConfigEmbed => PgFunc::json_build_object(vec![
                (
                    Expr::val("type").cast_as("text"),
                    Expr::col((HostEndpoints::Table, HostEndpoints::Type)),
                ),
                (
                    Expr::val("config").cast_as("text"),
                    Expr::col((HostEndpoints::Table, HostEndpoints::Config)),
                ),
            ])
            .into(),
            EndpointColumn::ResolvedAdmStatus => Func::cust(Least)
                .arg(Expr::col((HostEndpoints::Table, HostEndpoints::AdmStatus)))
                .arg(Expr::col((Hosts::Table, Hosts::AdmStatus)))
                .into(),
            EndpointColumn::MonitoredBy => Expr::col((Hosts::Table, Hosts::MonitoredBy)),
        }
    }
}

#[derive(Debug, Clone)]
pub struct EndpointParams {
    columns: Vec<EndpointColumn>,
    id_subquery_filter: Option<Expr>,
    only_enabled_resolved: bool,
    monitored_by: Option<Uuid>,
}

impl Default for EndpointParams {
    fn default() -> Self {
        Self::new()
    }
}

impl EndpointParams {
    pub fn new() -> Self {
        Self {
            columns: vec![],
            id_subquery_filter: None,
            only_enabled_resolved: false,
            monitored_by: None,
        }
    }

    pub fn with_columns(&mut self, columns: Vec<EndpointColumn>) -> &mut Self {
        self.columns = columns;
        self
    }

    pub fn with_id_subquery_filter(&mut self, filter: Option<Expr>) -> &mut Self {
        self.id_subquery_filter = filter;
        self
    }

    pub fn with_only_enabled_resolved(&mut self, only_enabled_resolved: bool) -> &mut Self {
        self.only_enabled_resolved = only_enabled_resolved;
        self
    }

    pub fn with_monitored_by(&mut self, monitored_by: Option<Uuid>) -> &mut Self {
        self.monitored_by = monitored_by;
        self
    }
}

pub fn endpoints(params: EndpointParams) -> SelectStatement {
    let mut query = Query::select();
    query.from(HostEndpoints::Table);
    for column in &params.columns {
        query.expr_as(*column, column.as_str());
    }
    if let Some(filter) = params.id_subquery_filter {
        query.and_where(EndpointColumn::EndpointId.binary(types::BinOper::In, filter));
    }
    if params.monitored_by.is_some()
        || params.only_enabled_resolved
        || params.columns.contains(&EndpointColumn::ResolvedAdmStatus)
        || params.columns.contains(&EndpointColumn::MonitoredBy)
    {
        query.join(
            JoinType::Join,
            Hosts::Table,
            Expr::col((HostEndpoints::Table, HostEndpoints::HostId))
                .eq(Expr::col((Hosts::Table, Hosts::HostId))),
        );
    }
    if let Some(monitored_by) = params.monitored_by {
        query.and_where(EndpointColumn::MonitoredBy.eq(monitored_by));
    }
    if params.only_enabled_resolved {
        query.and_where(EndpointColumn::ResolvedAdmStatus.eq(AdmStatus::Enabled as i16));
    }
    query
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ItemColumn {
    ItemId,
    HostTemplateId,
    TemplateItemId,
    TemplateItemName,
    ListenerId,
    ListenerName,
    EndpointId,
    EndpointName,
    Type,
    Flags,
    AdmStatus,
    Name,
    Schedule,
    TimeoutMs,
    LastUpdated,
    Config,
    Labels,

    ConfigEmbed,
    Outputs,
    HostId,
    HostName,
    MonitoredBy,
    TemplateId,
    TemplateName,

    ResolvedAdmStatus,
    ResolvedName,
    ResolvedSchedule,
    ResolvedTimeoutMs,
    ResolvedConfig,
    ResolvedConfigEmbed,
    ResolvedOutputs,
    RootItemId,
    RootTemplateId,
    RootTemplateName,
}

impl ItemColumn {
    pub fn as_str(&self) -> &'static str {
        match self {
            ItemColumn::ItemId => "item_id",
            ItemColumn::HostTemplateId => "host_template_id",
            ItemColumn::TemplateItemId => "template_item_id",
            ItemColumn::TemplateItemName => "template_item_name",
            ItemColumn::ListenerId => "listener_id",
            ItemColumn::ListenerName => "listener_name",
            ItemColumn::EndpointId => "endpoint_id",
            ItemColumn::EndpointName => "endpoint_name",
            ItemColumn::Type => "type",
            ItemColumn::Flags => "flags",
            ItemColumn::AdmStatus => "adm_status",
            ItemColumn::Name => "name",
            ItemColumn::Schedule => "schedule",
            ItemColumn::TimeoutMs => "timeout_ms",
            ItemColumn::LastUpdated => "last_updated",
            ItemColumn::Config => "config",
            ItemColumn::Labels => "labels",
            ItemColumn::ConfigEmbed => "config_embed",
            ItemColumn::Outputs => "outputs",
            ItemColumn::HostId => "host_id",
            ItemColumn::HostName => "host_name",
            ItemColumn::MonitoredBy => "monitored_by",
            ItemColumn::TemplateId => "template_id",
            ItemColumn::TemplateName => "template_name",
            ItemColumn::ResolvedAdmStatus => "resolved_adm_status",
            ItemColumn::ResolvedName => "resolved_name",
            ItemColumn::ResolvedSchedule => "resolved_schedule",
            ItemColumn::ResolvedTimeoutMs => "resolved_timeout_ms",
            ItemColumn::ResolvedConfig => "resolved_config",
            ItemColumn::ResolvedConfigEmbed => "resolved_config_embed",
            ItemColumn::ResolvedOutputs => "resolved_outputs",
            ItemColumn::RootItemId => "root_item_id",
            ItemColumn::RootTemplateId => "root_template_id",
            ItemColumn::RootTemplateName => "root_template_name",
        }
    }

    pub fn needs_resolve(&self) -> bool {
        matches!(
            self,
            ItemColumn::ResolvedName
                | ItemColumn::ResolvedSchedule
                | ItemColumn::ResolvedTimeoutMs
                | ItemColumn::ResolvedConfig
                | ItemColumn::ResolvedConfigEmbed
                | ItemColumn::ResolvedOutputs
                | ItemColumn::RootItemId
                | ItemColumn::RootTemplateId
                | ItemColumn::RootTemplateName
        )
    }
}

impl Display for ItemColumn {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

const ITEMS_RESOLVED_CTE: &str = "items_resolved_cte";
const ITEMS_RESOLVED_SUMMARY_CTE: &str = "items_resolved_summary_cte";

impl From<ItemColumn> for Expr {
    fn from(value: ItemColumn) -> Self {
        match value {
            ItemColumn::ItemId => Expr::col((Items::Table, Items::ItemId)),
            ItemColumn::HostTemplateId => Expr::col((Items::Table, Items::HostTemplateId)),
            ItemColumn::TemplateItemId => Expr::col((Items::Table, Items::TemplateItemId)),
            ItemColumn::TemplateItemName => Expr::SubQuery(
                None,
                Box::new(
                    SelectStatement::new()
                        .expr(Expr::col(("i", Items::Name)))
                        .from_as(Items::Table, "i")
                        .and_where(
                            Expr::col(("i", Items::ItemId))
                                .eq(Expr::col((Items::Table, Items::TemplateItemId))),
                        )
                        .to_owned()
                        .into_sub_query_statement(),
                ),
            ),
            ItemColumn::ListenerId => Expr::col((Items::Table, Items::ListenerId)),
            ItemColumn::ListenerName => Expr::col((Listeners::Table, Listeners::Name)),
            ItemColumn::EndpointId => Expr::col((Items::Table, Items::EndpointId)),
            ItemColumn::EndpointName => Expr::col((HostEndpoints::Table, HostEndpoints::Name)),
            ItemColumn::Type => Expr::col((Items::Table, Items::Type)),
            ItemColumn::Flags => Expr::col((Items::Table, Items::Flags)),
            ItemColumn::AdmStatus => Expr::col((Items::Table, Items::AdmStatus)),
            ItemColumn::Name => Expr::col((Items::Table, Items::Name)),
            ItemColumn::Schedule => Expr::col((Items::Table, Items::Schedule)),
            ItemColumn::TimeoutMs => Expr::col((Items::Table, Items::TimeoutMs)),
            ItemColumn::LastUpdated => Expr::col((Items::Table, Items::LastUpdated)),
            ItemColumn::Config => Expr::col((Items::Table, Items::Config)),
            ItemColumn::Labels => Expr::col((Items::Table, Items::Labels)),
            ItemColumn::ConfigEmbed => PgFunc::json_build_object(vec![
                (
                    Expr::val("type").cast_as("text"),
                    Expr::col((Items::Table, Items::Type)),
                ),
                (
                    Expr::val("config").cast_as("text"),
                    Expr::col((Items::Table, Items::Config)),
                ),
            ])
            .into(),
            ItemColumn::Outputs => Func::coalesce(vec![
                Expr::SubQuery(
                    None,
                    Box::new(
                        SelectStatement::new()
                            .expr(PgFunc::array_agg(Expr::col((
                                "ip",
                                ItemProcessors::ProcessorId,
                            ))))
                            .from_as(ItemProcessors::Table, "ip")
                            .and_where(
                                Expr::col(("ip", ItemProcessors::ItemId))
                                    .eq(Expr::col((Items::Table, Items::ItemId))),
                            )
                            .to_owned()
                            .into_sub_query_statement(),
                    ),
                ),
                Expr::cust("'{}'::uuid[]"),
            ])
            .into(),
            ItemColumn::HostId => Expr::col((Hosts::Table, Hosts::HostId)),
            ItemColumn::HostName => Expr::col((Hosts::Table, Hosts::Name)),
            ItemColumn::MonitoredBy => Expr::col((Hosts::Table, Hosts::MonitoredBy)),
            ItemColumn::TemplateId => Expr::col((Templates::Table, Templates::TemplateId)),
            ItemColumn::TemplateName => Expr::col((Templates::Table, Templates::Name)),
            ItemColumn::ResolvedAdmStatus => Func::cust(Least)
                .arg(Expr::col((Items::Table, Items::AdmStatus)))
                .arg(Expr::col((HostEndpoints::Table, HostEndpoints::AdmStatus)))
                .arg(Expr::col((Hosts::Table, Hosts::AdmStatus)))
                .into(),
            ItemColumn::ResolvedName => Expr::col((
                ITEMS_RESOLVED_SUMMARY_CTE,
                ItemColumn::ResolvedName.as_str(),
            )),
            ItemColumn::ResolvedSchedule => Expr::col((
                ITEMS_RESOLVED_SUMMARY_CTE,
                ItemColumn::ResolvedSchedule.as_str(),
            )),
            ItemColumn::ResolvedTimeoutMs => Expr::col((
                ITEMS_RESOLVED_SUMMARY_CTE,
                ItemColumn::ResolvedTimeoutMs.as_str(),
            )),
            ItemColumn::ResolvedConfig => Expr::col((
                ITEMS_RESOLVED_SUMMARY_CTE,
                ItemColumn::ResolvedConfig.as_str(),
            )),
            ItemColumn::ResolvedConfigEmbed => PgFunc::json_build_object(vec![
                (
                    Expr::val("type").cast_as("text"),
                    Expr::col((Items::Table, Items::Type)),
                ),
                (
                    Expr::val("config").cast_as("text"),
                    Expr::col((
                        ITEMS_RESOLVED_SUMMARY_CTE,
                        ItemColumn::ResolvedConfig.as_str(),
                    )),
                ),
            ])
            .into(),
            ItemColumn::ResolvedOutputs => Expr::col((
                ITEMS_RESOLVED_SUMMARY_CTE,
                ItemColumn::ResolvedOutputs.as_str(),
            )),
            ItemColumn::RootItemId => {
                Expr::col((ITEMS_RESOLVED_SUMMARY_CTE, ItemColumn::RootItemId.as_str()))
            }
            ItemColumn::RootTemplateId => Expr::col((
                ITEMS_RESOLVED_SUMMARY_CTE,
                ItemColumn::RootTemplateId.as_str(),
            )),
            ItemColumn::RootTemplateName => Expr::col((
                ITEMS_RESOLVED_SUMMARY_CTE,
                ItemColumn::RootTemplateName.as_str(),
            )),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ItemParams {
    columns: Vec<ItemColumn>,
    id_subquery_filter: Option<Expr>,
    only_enabled_resolved: bool,
    monitored_by: Option<Uuid>,
    // only items that are attached to a host. basically excluding template items
    only_monitored: bool,
    additional_filters: Vec<Expr>,
    additional_filters_last: Vec<Expr>,
    limit: u64,
    offset: u64,
    order_bys: Vec<(Expr, Order)>,
}

impl Default for ItemParams {
    fn default() -> Self {
        Self::new()
    }
}

impl ItemParams {
    pub fn new() -> Self {
        Self {
            columns: vec![],
            id_subquery_filter: None,
            only_enabled_resolved: false,
            monitored_by: None,
            only_monitored: false,
            additional_filters: vec![],
            additional_filters_last: vec![],
            limit: 0,
            offset: 0,
            order_bys: vec![],
        }
    }

    pub fn with_columns(&mut self, columns: Vec<ItemColumn>) -> &mut Self {
        self.columns = columns;
        self
    }

    pub fn with_id_subquery_filter(&mut self, filter: Option<Expr>) -> &mut Self {
        self.id_subquery_filter = filter;
        self
    }

    pub fn with_only_enabled_resolved(&mut self, only_enabled_resolved: bool) -> &mut Self {
        self.only_enabled_resolved = only_enabled_resolved;
        self
    }

    pub fn with_monitored_by(&mut self, monitored_by: Option<Uuid>) -> &mut Self {
        self.monitored_by = monitored_by;
        self
    }

    pub fn with_only_monitored(&mut self, only_monitored: bool) -> &mut Self {
        self.only_monitored = only_monitored;
        self
    }

    pub fn with_additional_filters(&mut self, additional_filters: Vec<Expr>) -> &mut Self {
        self.additional_filters = additional_filters;
        self
    }

    pub fn with_additional_filters_last(
        &mut self,
        additional_filters_last: Vec<Expr>,
    ) -> &mut Self {
        self.additional_filters_last = additional_filters_last;
        self
    }

    pub fn with_limit(&mut self, limit: u64) -> &mut Self {
        self.limit = limit;
        self
    }

    pub fn with_offset(&mut self, offset: u64) -> &mut Self {
        self.offset = offset;
        self
    }

    pub fn with_order_bys(&mut self, order_bys: Vec<(Expr, Order)>) -> &mut Self {
        self.order_bys = order_bys;
        self
    }
}

// item filtering happens here
fn items_base_select(params: ItemParams, always_join: bool) -> SelectStatement {
    let mut query = Query::select();
    query.from(Items::Table);
    if always_join
        || params.only_monitored
        || params.only_enabled_resolved
        || params.monitored_by.is_some()
        || params.columns.contains(&ItemColumn::TemplateId)
        || params.columns.contains(&ItemColumn::TemplateName)
    {
        query.join(
            JoinType::LeftJoin,
            HostEndpoints::Table,
            Expr::col((Items::Table, Items::EndpointId))
                .eq(Expr::col((HostEndpoints::Table, HostEndpoints::EndpointId))),
        );
        query.join(
            JoinType::LeftJoin,
            HostTemplates::Table,
            Expr::col((Items::Table, Items::HostTemplateId)).eq(Expr::col((
                HostTemplates::Table,
                HostTemplates::HostTemplateId,
            ))),
        );
        query.join(
            JoinType::LeftJoin,
            Hosts::Table,
            Expr::col((HostTemplates::Table, HostTemplates::HostId))
                .eq(Expr::col((Hosts::Table, Hosts::HostId))),
        );
        query.join(
            JoinType::LeftJoin,
            Templates::Table,
            Expr::col((HostTemplates::Table, HostTemplates::TemplateId))
                .eq(Expr::col((Templates::Table, Templates::TemplateId))),
        );
    }

    if params.columns.contains(&ItemColumn::ListenerName) {
        query.join(
            JoinType::LeftJoin,
            Listeners::Table,
            Expr::col((Items::Table, Items::ListenerId))
                .eq(Expr::col((Listeners::Table, Listeners::ListenerId))),
        );
    }

    if let Some(filter) = params.id_subquery_filter {
        query.and_where(ItemColumn::ItemId.binary(BinOper::In, filter));
    }
    if let Some(monitored_by) = params.monitored_by {
        query.and_where(ItemColumn::MonitoredBy.eq(Expr::val(monitored_by).cast_as("uuid")));
    }
    if params.only_monitored {
        query.and_where(ItemColumn::HostId.is_not_null());
    }
    if params.only_enabled_resolved {
        query.and_where(ItemColumn::ResolvedAdmStatus.eq(AdmStatus::Enabled as i16));
    }
    for filter in params.additional_filters {
        query.and_where(filter);
    }
    for (expr, order) in params.order_bys {
        query.order_by_expr(expr, order);
    }
    query
}

fn items_recursive_select() -> SelectStatement {
    let mut query = Query::select();
    query.from(Items::Table);
    query.join(
        JoinType::Join,
        ITEMS_RESOLVED_CTE,
        Expr::col((Items::Table, Items::ItemId)).eq(Expr::col((
            ITEMS_RESOLVED_CTE,
            ItemColumn::TemplateItemId.as_str(),
        ))),
    );
    query.join(
        JoinType::LeftJoin,
        HostTemplates::Table,
        Expr::col((Items::Table, Items::HostTemplateId)).eq(Expr::col((
            HostTemplates::Table,
            HostTemplates::HostTemplateId,
        ))),
    );
    query.join(
        JoinType::LeftJoin,
        Templates::Table,
        Expr::col((HostTemplates::Table, HostTemplates::TemplateId))
            .eq(Expr::col((Templates::Table, Templates::TemplateId))),
    );
    query
}

fn items_resolved_cte(params: ItemParams) -> CommonTableExpression {
    let mut base_query = items_base_select(params, true);
    base_query
        .expr_as(ItemColumn::ItemId, ItemColumn::ItemId.as_str())
        .expr_as(
            ItemColumn::TemplateItemId,
            ItemColumn::TemplateItemId.as_str(),
        )
        .expr_as(ItemColumn::Name, ItemColumn::ResolvedName.as_str())
        .expr_as(ItemColumn::Schedule, ItemColumn::ResolvedSchedule.as_str())
        .expr_as(
            ItemColumn::TimeoutMs,
            ItemColumn::ResolvedTimeoutMs.as_str(),
        )
        .expr_as(ItemColumn::Config, ItemColumn::ResolvedConfig.as_str())
        .expr_as(
            Expr::SubQuery(
                None,
                Box::new(
                    SelectStatement::new()
                        .expr(PgFunc::array_agg(Expr::col((
                            "ip",
                            ItemProcessors::ProcessorId,
                        ))))
                        .from_as(ItemProcessors::Table, "ip")
                        .and_where(Expr::col(("ip", ItemProcessors::ItemId)).eq(ItemColumn::ItemId))
                        .to_owned()
                        .into_sub_query_statement(),
                ),
            ),
            ItemColumn::ResolvedOutputs.as_str(),
        )
        .expr_as(ItemColumn::ItemId, ItemColumn::RootItemId.as_str())
        .expr_as(
            Expr::col((Templates::Table, Templates::TemplateId)),
            ItemColumn::RootTemplateId.as_str(),
        )
        .expr_as(
            Expr::col((Templates::Table, Templates::Name)),
            ItemColumn::RootTemplateName.as_str(),
        )
        .expr_as(
            Expr::cust_with_expr("array_append(array[]::uuid[], $1)", ItemColumn::ItemId),
            "path",
        );
    let mut base_query = Query::select()
        .from_subquery(base_query, "base")
        .column(Asterisk)
        .to_owned();
    let mut recursive_query = items_recursive_select();
    recursive_query
        .expr(Expr::col((ITEMS_RESOLVED_CTE, ItemColumn::ItemId.as_str())))
        .expr(ItemColumn::TemplateItemId)
        .expr(Func::coalesce(vec![
            Expr::col((ITEMS_RESOLVED_CTE, ItemColumn::ResolvedName.as_str())),
            ItemColumn::Name.into(),
        ]))
        .expr(Func::coalesce(vec![
            Expr::col((ITEMS_RESOLVED_CTE, ItemColumn::ResolvedSchedule.as_str())),
            ItemColumn::Schedule.into(),
        ]))
        .expr(Func::coalesce(vec![
            Expr::col((ITEMS_RESOLVED_CTE, ItemColumn::ResolvedTimeoutMs.as_str())),
            ItemColumn::TimeoutMs.into(),
        ]))
        .expr(Func::coalesce(vec![
            Expr::col((ITEMS_RESOLVED_CTE, ItemColumn::ResolvedConfig.as_str())),
            ItemColumn::Config.into(),
        ]))
        .expr(Func::coalesce(vec![
            Expr::col((ITEMS_RESOLVED_CTE, ItemColumn::ResolvedOutputs.as_str())),
            Expr::SubQuery(
                None,
                Box::new(
                    SelectStatement::new()
                        .expr(PgFunc::array_agg(Expr::col((
                            "ip",
                            ItemProcessors::ProcessorId,
                        ))))
                        .from_as(ItemProcessors::Table, "ip")
                        .and_where(Expr::col(("ip", ItemProcessors::ItemId)).eq(ItemColumn::ItemId))
                        .to_owned()
                        .into_sub_query_statement(),
                ),
            ),
        ]))
        .expr(ItemColumn::ItemId)
        .expr(Expr::col((Templates::Table, Templates::TemplateId)))
        .expr(Expr::col((Templates::Table, Templates::Name)))
        .expr(Expr::cust_with_exprs(
            "$1 || $2",
            vec![
                Expr::col((ITEMS_RESOLVED_CTE, "path")),
                ItemColumn::ItemId.into(),
            ],
        ))
        .and_where(
            ItemColumn::ItemId
                .eq(PgFunc::any(Expr::col((ITEMS_RESOLVED_CTE, "path"))))
                .not(),
        );
    let mut cte = CommonTableExpression::new();
    cte.query(base_query.union(UnionType::All, recursive_query).to_owned())
        .table_name(ITEMS_RESOLVED_CTE);
    cte
}

fn items_resolved_summary_cte() -> CommonTableExpression {
    let mut query = Query::select();
    query.from(ITEMS_RESOLVED_CTE);
    query
        .expr(Expr::col(ItemColumn::ItemId.as_str()))
        .expr(Expr::col(ItemColumn::ResolvedName.as_str()))
        .expr(Expr::col(ItemColumn::ResolvedSchedule.as_str()))
        .expr(Expr::col(ItemColumn::ResolvedTimeoutMs.as_str()))
        .expr(Expr::col(ItemColumn::ResolvedConfig.as_str()))
        .expr_as(
            Func::coalesce(vec![
                Expr::col(ItemColumn::ResolvedOutputs.as_str()),
                Expr::cust("'{}'::uuid[]"),
            ]),
            ItemColumn::ResolvedOutputs.as_str(),
        )
        .expr(Expr::col(ItemColumn::RootItemId.as_str()))
        .expr(Expr::col(ItemColumn::RootTemplateId.as_str()))
        .expr(Expr::col(ItemColumn::RootTemplateName.as_str()))
        .distinct_on(vec![ItemColumn::ItemId.as_str()])
        .order_by(ItemColumn::ItemId.as_str(), Order::Asc)
        .order_by_expr(
            Func::cust(ArrayLength).arg(Expr::col("path")).arg(1).into(),
            Order::Desc,
        );
    let mut cte = CommonTableExpression::new();
    cte.query(query).table_name(ITEMS_RESOLVED_SUMMARY_CTE);
    cte
}

pub enum ItemQuery {
    Select(Box<SelectStatement>),
    Cte(Box<WithQuery>),
}

impl PostgresBinder for ItemQuery {
    fn build_postgres<T: QueryBuilder>(&self, query_builder: T) -> (String, PostgresValues) {
        match self {
            ItemQuery::Select(query) => query.build_postgres(query_builder),
            ItemQuery::Cte(query) => query.build_postgres(query_builder),
        }
    }
}

impl Display for ItemQuery {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ItemQuery::Select(query) => write!(f, "{}", query.to_string(PostgresQueryBuilder)),
            ItemQuery::Cte(query) => write!(f, "{}", query.to_string(PostgresQueryBuilder)),
        }
    }
}

pub fn items(params: ItemParams) -> ItemQuery {
    let mut with_clause = WithClause::new();
    let mut query = items_base_select(params.clone(), false);
    let mut has_cte = false;
    if params.columns.iter().any(|c| c.needs_resolve()) {
        with_clause
            .recursive(true)
            .cte(items_resolved_cte(params.clone()))
            .cte(items_resolved_summary_cte());
        query.join(
            JoinType::Join,
            ITEMS_RESOLVED_SUMMARY_CTE,
            Expr::col((Items::Table, Items::ItemId))
                .eq(Expr::col((ITEMS_RESOLVED_SUMMARY_CTE, Items::ItemId))),
        );
        has_cte = true;
    }
    for column in &params.columns {
        query.expr_as(*column, column.as_str());
    }

    if !params.additional_filters_last.is_empty() {
        for filter in params.additional_filters_last {
            query.and_where(filter);
        }
    }

    if params.limit > 0 {
        query.limit(params.limit);
    }
    if params.offset > 0 {
        query.offset(params.offset);
    }

    if has_cte {
        ItemQuery::Cte(Box::new(query.with(with_clause)))
    } else {
        ItemQuery::Select(Box::new(query))
    }
}

pub struct ClusterLogParams {
    pub node_id: Uuid,
    resources: Vec<ClusterLogResource>,
    columns: Vec<ClusterLogs>,
    op: Option<ClusterLogsOp>,
}

impl ClusterLogParams {
    pub fn new(node_id: Uuid) -> Self {
        Self {
            node_id,
            resources: Vec::new(),
            columns: Vec::new(),
            op: None,
        }
    }

    pub fn with_columns(&mut self, columns: Vec<ClusterLogs>) -> &mut Self {
        self.columns = columns;
        self
    }

    pub fn with_resources(&mut self, resources: Vec<ClusterLogResource>) -> &mut Self {
        self.resources = resources;
        self
    }

    pub fn with_op(&mut self, op: ClusterLogsOp) -> &mut Self {
        self.op = Some(op);
        self
    }
}

pub fn cluster_logs(params: ClusterLogParams) -> WithQuery {
    let mut select = Query::select();
    select
        .from(ClusterLogs::Table)
        .column(Asterisk)
        .and_where(Expr::col(ClusterLogs::NodeId).eq(params.node_id))
        .distinct_on(vec![ClusterLogs::Resource, ClusterLogs::ResourceId])
        .order_by(ClusterLogs::Resource, Order::Asc)
        .order_by(ClusterLogs::ResourceId, Order::Asc)
        // latest row per resource; one transaction's rows all tie on `Ts`
        .order_by(ClusterLogs::Seq, Order::Desc);
    if !params.resources.is_empty() {
        select.and_where(
            Expr::col(ClusterLogs::Resource).is_in(
                params
                    .resources
                    .iter()
                    .map(|r| r.to_string())
                    .collect::<Vec<_>>(),
            ),
        );
    }
    let mut select_summary = Query::select();
    select_summary
        .from("cluster_log_summary")
        .columns(params.columns);
    if let Some(op) = params.op {
        select_summary.and_where(Expr::col(ClusterLogs::Op).eq(op as i16));
    }
    WithQuery::new()
        .cte(
            CommonTableExpression::new()
                .query(select)
                .table_name("cluster_log_summary")
                .to_owned(),
        )
        .query(select_summary)
        .to_owned()
}

/// Wraps a given select statement in a CTE and generates a new select statement that counts the total number of rows in the original query.
pub fn generate_count_query(base_select: &SelectStatement) -> SelectStatement {
    let mut query_cte = CommonTableExpression::from_select(base_select.clone());
    query_cte.table_name("__base_query");
    let mut total_query_select = SelectStatement::new();
    total_query_select
        .expr_as(Expr::cust("COUNT(*)"), "total")
        .from(Alias::new("__base_query"));

    total_query_select.with_cte(query_cte.clone()).to_owned()
}

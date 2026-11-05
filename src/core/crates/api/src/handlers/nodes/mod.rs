// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

pub mod list_nodes;

pub use list_nodes::list_nodes;

use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    paths(list_nodes::list_nodes,),
    components(schemas(
        list_nodes::ResponseClusterNode,
        list_nodes::ListNodesResponse,
        list_nodes::NodeStatus
    ))
)]
pub struct NodesHandler;

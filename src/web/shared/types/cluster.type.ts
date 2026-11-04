// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { ResponseClusterNodeOutput } from '@@/shared/generated/patrolliAPI.schemas.type'
import type { IBaseResponse } from '@@/shared/types/api.type'

export interface ICluster {
	key?: string
	name: string
	url: string
	clientPath?: string
}

export interface IKeycloakConfig {
	host: string
	client_id: string
	client_secret: string
	redirect_uri: string
	ca_cert_path?: string
}

export interface ICorsConfig {
	enabled?: boolean
	origin?: string | string[]
	origins?: string | string[]
	methods?: string | string[]
	allow_headers?: string | string[]
	headers?: string | string[]
	expose_headers?: string | string[]
	credentials?: boolean
	max_age?: number
}

export interface IClusterSelect extends ICluster {
	label: string
	value: string
}

export interface IClusterListResponse extends IBaseResponse {
	data: IClusterSelect[]
}

export interface IClusterNode extends ResponseClusterNodeOutput {}

export interface IClusterNodeSelect extends ResponseClusterNodeOutput {
	label?: string
	value?: string
}

export interface IClusterNodeListResponse extends IBaseResponse {
	data: {
		nodes: IClusterNode[]
	}
}

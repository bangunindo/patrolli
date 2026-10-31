// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { ICluster } from '@@/shared/types/cluster.type'

export interface IProxyRequestOptions {
	method: string
	headers?: Record<string, any>
	query?: Record<string, any>
	body?: any
}

export interface IProxyClusterResponse {
	cluster: ICluster
	data: any
	status: number
	headers?: Headers
}

export interface IPaginationPlan {
	cluster: ICluster
	localQuery: Record<string, any>
	localOffset: number
	localLimit: number
}

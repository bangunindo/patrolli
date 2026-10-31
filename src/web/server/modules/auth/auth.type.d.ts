// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { ICluster } from '@@/shared/types/cluster.type'

export type AuthProvider = 'local' | 'keycloak'

export interface IAuthCredentials {
	username?: string
	password?: string
	[key: string]: any
}

export interface IAuthRefreshPayload {
	refresh_token?: string
	auth_provider?: AuthProvider
}

export interface IAuthClusterResult {
	cluster: ICluster
	data: any
	status: number
	headers?: Headers
}

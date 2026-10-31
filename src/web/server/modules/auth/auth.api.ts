// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { ICluster } from '@@/shared/types/cluster.type'
import { forwardToCluster } from '@@/server/services/cluster.service'

/**
 * Forwards an authentication request (login, logout, refresh) to a specific cluster node
 */
export async function forwardAuthToCluster(
	cluster: ICluster,
	path: string,
	options: {
		method: string
		headers?: Record<string, any>
		query?: Record<string, any>
		body?: any
	}
) {
	return forwardToCluster(cluster, path, options)
}

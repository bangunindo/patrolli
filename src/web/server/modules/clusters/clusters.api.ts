// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { ICluster } from '@@/shared/types/cluster.type'
import { forwardToCluster } from '@@/server/services/cluster.service'

/**
 * Pings or fetches health status from a specific cluster node
 */
export async function pingClusterNode(cluster: ICluster) {
	return forwardToCluster(cluster, '/api/health', {
		method: 'GET'
	})
}

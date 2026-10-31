// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { ICluster } from '@@/shared/types/cluster.type'
import { forwardToCluster } from '@@/server/services/cluster.service'
import type { IProxyRequestOptions } from './proxy.type'

/**
 * Forwards an HTTP request to a target cluster node
 */
export async function forwardProxyToCluster(
	cluster: ICluster,
	path: string,
	options: IProxyRequestOptions
) {
	return forwardToCluster(cluster, path, options)
}

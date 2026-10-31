// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { H3Event } from 'h3'
import { forwardToCluster, getClustersConfig } from '@@/server/services/cluster.service'
import { formatClusterList } from '@@/server/modules/clusters/clusters.utils'
import type { IClusterClientItem } from '@@/server/modules/clusters/clusters.type'
import { setUserClustersCookie } from '@@/server/utils/auth-cookies'

/**
 * Verifies which clusters the authenticated user has access to by querying GET /api/nodes
 * on each configured cluster. If /api/nodes returns HTTP 200, access is granted.
 * Sets the resulting cluster list in the `user_clusters` cookie.
 */
export async function verifyUserClusterAccess(
	event: H3Event,
	accessToken: string
): Promise<IClusterClientItem[]> {
	if (!accessToken) {
		setUserClustersCookie(event, [])
		return []
	}

	const allClusters = getClustersConfig(event)
	if (!allClusters || allClusters.length === 0) {
		setUserClustersCookie(event, [])
		return []
	}

	const checkResults = await Promise.all(
		allClusters.map(async (cluster) => {
			try {
				const res = await forwardToCluster(cluster, '/api/nodes', {
					method: 'GET',
					headers: {
						authorization: `Bearer ${accessToken}`
					}
				})
				return {
					cluster,
					hasAccess: res.status === 200
				}
			} catch {
				return {
					cluster,
					hasAccess: false
				}
			}
		})
	)

	const accessibleClusters = checkResults
		.filter((result) => result.hasAccess)
		.map((result) => result.cluster)

	const formatted = formatClusterList(accessibleClusters)
	setUserClustersCookie(event, formatted)
	return formatted
}

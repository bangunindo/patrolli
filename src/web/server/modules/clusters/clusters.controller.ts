// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { verifyUserClusterAccess } from '@@/server/services/cluster-access.service'
import { getClustersConfig } from '@@/server/services/cluster.service'
import type { H3Event } from 'h3'
import { getCookie, getQuery, getRequestHeaders } from 'h3'
import type { IClustersResponse } from './clusters.type'
import { formatClusterList } from './clusters.utils'

/**
 * Handles GET /api/clusters listing available clusters for the frontend.
 * When authenticated, filters to only clusters the user has access to.
 */
export async function handleGetClusters(event: H3Event): Promise<IClustersResponse> {
	const allClusters = getClustersConfig(event)
	const query = getQuery(event)
	const forceRefresh = query.refresh === 'true' || query.refresh === true

	// 1. Check if user_clusters cookie is already present
	const userClustersCookie = !forceRefresh ? getCookie(event, 'user_clusters') : null
	if (userClustersCookie) {
		try {
			const parsed =
				typeof userClustersCookie === 'string'
					? JSON.parse(decodeURIComponent(userClustersCookie))
					: userClustersCookie
			if (Array.isArray(parsed) && parsed.length > 0) {
				return {
					status_code: 200,
					success: true,
					data: parsed
				}
			}
		} catch {
			/* empty */
		}
	}

	// 2. If user is authenticated via cookie or Authorization header, verify access via /api/nodes
	const accessToken =
		getCookie(event, 'access_token') ||
		getRequestHeaders(event).authorization?.replace(/^Bearer\s+/i, '')
	if (accessToken) {
		try {
			const accessible = await verifyUserClusterAccess(event, accessToken)
			if (accessible && accessible.length > 0) {
				return {
					status_code: 200,
					success: true,
					data: accessible
				}
			}
		} catch {
			/* empty */
		}
	}

	// 3. Fallback to all configured clusters (e.g. unauthenticated / login screen)
	return {
		status_code: 200,
		success: true,
		data: formatClusterList(allClusters)
	}
}

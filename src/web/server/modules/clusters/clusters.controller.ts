// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { tryTransparentRefreshOn401 } from '@@/server/services/auth-refresh.service'
import { verifyUserClusterAccessWithStatus } from '@@/server/services/cluster-access.service'
import { getClustersConfig } from '@@/server/services/cluster.service'
import { getAuthCookies, isJwtExpired } from '@@/server/utils/auth-cookies'
import type { H3Event } from 'h3'
import { getCookie, getQuery, getRequestHeaders } from 'h3'
import type { IClustersResponse } from '@@/shared/types/cluster.type'
import { formatClusterList } from './clusters.utils'

/**
 * Handles GET /api/clusters listing available clusters for the frontend.
 * When authenticated, filters to only clusters the user has access to.
 */
export async function handleGetClusters(event: H3Event): Promise<IClustersResponse> {
	const allClusters = getClustersConfig(event)
	const query = getQuery(event)
	const forceRefresh = query.refresh === 'true' || query.refresh === true

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
			// Invalid cookie JSON, fall through to re-verify cluster access
		}
	}

	const cookies = getAuthCookies(event)
	let accessToken =
		cookies.access_token || getRequestHeaders(event).authorization?.replace(/^Bearer\s+/i, '')
	let didRefresh = false

	if ((!accessToken || isJwtExpired(accessToken)) && cookies.refresh_token) {
		const refreshed = await tryTransparentRefreshOn401(event, cookies.active_cluster_url)
		if (refreshed) {
			accessToken = refreshed
			didRefresh = true
		}
	}

	if (accessToken) {
		try {
			let accessResult = await verifyUserClusterAccessWithStatus(event, accessToken)
			if (
				accessResult.clusters.length === 0 &&
				accessResult.sawUnauthorized &&
				!didRefresh &&
				cookies.refresh_token
			) {
				const refreshed = await tryTransparentRefreshOn401(
					event,
					cookies.active_cluster_url
				)
				if (refreshed) {
					accessResult = await verifyUserClusterAccessWithStatus(event, refreshed)
				}
			}
			if (accessResult.clusters.length > 0) {
				return {
					status_code: 200,
					success: true,
					data: accessResult.clusters
				}
			}
		} catch {
			// Verification failed, fall back to configured cluster list
		}
	}

	return {
		status_code: 200,
		success: true,
		data: formatClusterList(allClusters)
	}
}

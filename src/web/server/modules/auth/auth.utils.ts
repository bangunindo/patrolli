// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { H3Event } from 'h3'
import { getCookie, getQuery, getRequestHeaders } from 'h3'
import type { AuthProvider } from './auth.type'

/**
 * Extracts auth credentials & refresh tokens from request cookies or payload
 */
export function extractAuthRefreshParams(event: H3Event, body?: any) {
	const reqHeaders = getRequestHeaders(event)
	const query = getQuery(event)

	const refreshToken = getCookie(event, 'refresh_token') || body?.refresh_token || null
	const authProvider: AuthProvider =
		(getCookie(event, 'auth_provider') as AuthProvider) || body?.auth_provider || 'local'

	const clusterHeader =
		(reqHeaders['x-cluster-url'] as string | undefined) ||
		(getCookie(event, 'active_cluster_url') as string | undefined) ||
		(query.cluster as string | undefined) ||
		null

	return {
		refreshToken,
		authProvider,
		clusterHeader,
		query,
		reqHeaders
	}
}

/**
 * Checks whether a path parameter corresponds to a logout request
 */
export function isLogoutPath(pathParam: string, fullAuthPath: string): boolean {
	return (
		pathParam === 'logout' ||
		pathParam.replace(/\/$/, '') === 'logout' ||
		fullAuthPath.startsWith('/auth/logout')
	)
}

// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { H3Event } from 'h3'
import { getCookie, getQuery, getRequestHeaders } from 'h3'
import { normalizeAuthProvider } from '@@/shared/utils/auth'
import type { AuthProvider } from '@@/shared/types/auth.type'

export function extractAuthRefreshParams(event: H3Event, body?: any) {
	const reqHeaders = getRequestHeaders(event)
	const query = getQuery(event)

	const refreshToken = getCookie(event, 'refresh_token') || body?.refresh_token || null
	const authProvider: AuthProvider =
		normalizeAuthProvider(getCookie(event, 'auth_provider') || body?.auth_provider) || 'local'

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

export function isLoginPath(pathParam: string, fullAuthPath: string): boolean {
	return pathParam.replace(/\/$/, '') === 'login' || fullAuthPath.includes('/auth/login')
}

export function isLogoutPath(pathParam: string, fullAuthPath: string): boolean {
	return pathParam.replace(/\/$/, '') === 'logout' || fullAuthPath.includes('/auth/logout')
}

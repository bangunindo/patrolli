// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { type H3Event, deleteCookie, getCookie, parseCookies, setCookie } from 'h3'
import { isRequestSecure } from '@@/server/services/ssl.service'

export interface IAuthTokens {
	access_token?: string
	refresh_token?: string
	id_token?: string
}

export function setAuthCookies(
	event: H3Event,
	tokens: IAuthTokens,
	provider: 'local' | 'keycloak' = 'local',
	activeClusterUrl?: string | null
) {
	const isSecure = isRequestSecure(event)

	if (tokens.access_token) {
		setCookie(event, 'access_token', tokens.access_token, {
			path: '/',
			sameSite: 'lax',
			secure: isSecure,
			maxAge: 60 * 60 * 24 // 1 day
		})
	}

	if (tokens.refresh_token) {
		setCookie(event, 'refresh_token', tokens.refresh_token, {
			httpOnly: true,
			path: '/',
			sameSite: 'lax',
			secure: isSecure,
			maxAge: 60 * 60 * 24 * 7 // 7 days
		})
	}

	if (tokens.id_token) {
		setCookie(event, 'id_token', tokens.id_token, {
			path: '/',
			sameSite: 'lax',
			secure: isSecure,
			maxAge: 60 * 60 * 24 // 1 day
		})
	}

	setCookie(event, 'auth_provider', provider, {
		path: '/',
		sameSite: 'lax',
		secure: isSecure,
		maxAge: 60 * 60 * 24 * 7 // 7 days
	})

	setCookie(event, 'is_authenticated', 'true', {
		path: '/',
		sameSite: 'lax',
		secure: isSecure,
		maxAge: 60 * 60 * 24 // 1 day
	})

	if (activeClusterUrl) {
		setCookie(event, 'active_cluster_url', activeClusterUrl, {
			path: '/',
			sameSite: 'lax',
			secure: isSecure,
			maxAge: 60 * 60 * 24 * 7 // 7 days
		})
	}
}

export function getAuthCookies(event: H3Event) {
	return {
		access_token: getCookie(event, 'access_token'),
		refresh_token: getCookie(event, 'refresh_token'),
		id_token: getCookie(event, 'id_token'),
		auth_provider:
			(getCookie(event, 'auth_provider') as 'local' | 'keycloak' | undefined) || null,
		is_authenticated: getCookie(event, 'is_authenticated') === 'true',
		active_cluster_url: getCookie(event, 'active_cluster_url') || null
	}
}

export function setUserClustersCookie(event: H3Event, clusters: any[]) {
	const isSecure = isRequestSecure(event) && process.env.NODE_ENV === 'production'
	setCookie(event, 'user_clusters', JSON.stringify(clusters || []), {
		path: '/',
		sameSite: 'lax',
		secure: isSecure,
		httpOnly: false,
		maxAge: 60 * 60 * 24 * 7 // 7 days
	})
}

export function clearAuthCookies(event: H3Event) {
	const paths = ['/', '/auth', '/api']
	const authCookieNames = [
		'access_token',
		'refresh_token',
		'id_token',
		'auth_provider',
		'is_authenticated',
		'active_cluster_url',
		'user_clusters'
	]

	for (const name of authCookieNames) {
		for (const p of paths) {
			deleteCookie(event, name, { path: p })
		}
	}

	// Also clear any other dynamic session cookies (e.g., patrolli_*)
	const cookies = parseCookies(event)
	const allCookieNames = Object.keys(cookies)
	for (const name of allCookieNames) {
		for (const p of paths) {
			deleteCookie(event, name, { path: p })
		}
	}
}

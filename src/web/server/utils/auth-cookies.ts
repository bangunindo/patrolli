// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { type H3Event, deleteCookie, getCookie, parseCookies, setCookie } from 'h3'
import { isRequestSecure } from '@@/server/services/ssl.service'
import type { IAuthUser } from '@@/shared/types/auth.type'

const ONE_DAY_SECONDS = 60 * 60 * 24
const SEVEN_DAYS_SECONDS = ONE_DAY_SECONDS * 7

export interface IAuthTokens {
	access_token?: string
	refresh_token?: string
	id_token?: string
	expires_in?: number
}

export function extractTokenPayload(raw: any): IAuthTokens {
	if (!raw || typeof raw !== 'object') return {}
	const source =
		raw.data && typeof raw.data === 'object' && !Array.isArray(raw.data) ? raw.data : raw
	return {
		access_token: source.access_token || source.token,
		refresh_token: source.refresh_token,
		id_token: source.id_token,
		expires_in: source.expires_in
	}
}

export function decodeJwtPayload(token?: string | null): Record<string, any> | null {
	if (!token || typeof token !== 'string') return null
	const parts = token.split('.')
	if (parts.length < 2 || !parts[1]) return null
	try {
		const json = Buffer.from(parts[1], 'base64url').toString('utf-8')
		const parsed = JSON.parse(json)
		return parsed && typeof parsed === 'object' ? parsed : null
	} catch {
		return null
	}
}

export function extractJwtExpirationMs(token?: string | null): number | null {
	const payload = decodeJwtPayload(token)
	if (!payload || typeof payload.exp !== 'number') return null
	return payload.exp * 1000
}

export function isJwtExpired(token?: string | null, leewaySecs = 30): boolean {
	const expMs = extractJwtExpirationMs(token)
	if (expMs === null) return false
	return Date.now() >= expMs - leewaySecs * 1000
}

export function resolveAccessTokenExpiresAtMs(tokens?: IAuthTokens): number | null {
	if (!tokens) return null
	const jwtExpMs = extractJwtExpirationMs(tokens.access_token)
	if (jwtExpMs !== null) return jwtExpMs
	if (
		typeof tokens.expires_in === 'number' &&
		Number.isFinite(tokens.expires_in) &&
		tokens.expires_in > 0
	) {
		return Date.now() + tokens.expires_in * 1000
	}
	return null
}

export function extractRemainingTokenSeconds(token?: string | null): number | undefined {
	const expMs = extractJwtExpirationMs(token)
	if (expMs === null) return undefined
	return Math.max(0, Math.floor((expMs - Date.now()) / 1000))
}

export function extractAuthUserFromTokens(
	accessToken?: string | null,
	idToken?: string | null
): IAuthUser | null {
	const at = decodeJwtPayload(accessToken)
	const it = decodeJwtPayload(idToken)
	if (!at && !it) return null

	const preferred_username = at?.preferred_username || it?.preferred_username
	const username = at?.username || it?.username || preferred_username
	const sub = at?.sub || it?.sub
	const email = at?.email || it?.email
	const clusters = Array.isArray(at?.clusters)
		? at.clusters
		: Array.isArray(it?.clusters)
			? it.clusters
			: undefined
	const cluster =
		typeof at?.cluster === 'string'
			? at.cluster
			: typeof it?.cluster === 'string'
				? it.cluster
				: undefined

	return {
		...(preferred_username ? { preferred_username } : {}),
		...(username ? { username } : {}),
		...(sub ? { sub } : {}),
		...(email ? { email } : {}),
		...(clusters ? { clusters } : {}),
		...(cluster ? { cluster } : {})
	}
}

export function buildAuthSessionPayload(tokens?: IAuthTokens) {
	const user = extractAuthUserFromTokens(tokens?.access_token, tokens?.id_token)
	const expiresIn =
		tokens?.expires_in !== undefined
			? tokens.expires_in
			: extractRemainingTokenSeconds(tokens?.access_token)
	return {
		...(expiresIn !== undefined ? { expires_in: expiresIn } : {}),
		...(user ? { user } : {})
	}
}

export function setAuthCookies(
	event: H3Event,
	tokens: IAuthTokens,
	provider: 'local' | 'keycloak' = 'local',
	activeClusterUrl?: string | null
) {
	const isSecure = isRequestSecure(event)
	const unwrapped = extractTokenPayload(tokens)
	const baseOptions = {
		path: '/',
		sameSite: 'lax' as const,
		secure: isSecure
	}

	if (unwrapped.access_token) {
		setCookie(event, 'access_token', unwrapped.access_token, {
			...baseOptions,
			httpOnly: true,
			maxAge: ONE_DAY_SECONDS
		})

		const expiresAtMs = resolveAccessTokenExpiresAtMs(unwrapped)
		if (expiresAtMs !== null) {
			setCookie(event, 'access_token_exp', String(expiresAtMs), {
				...baseOptions,
				httpOnly: false,
				maxAge: ONE_DAY_SECONDS
			})
		}

		setCookie(event, 'auth_refreshed_at', String(Date.now()), {
			...baseOptions,
			httpOnly: false,
			maxAge: ONE_DAY_SECONDS
		})
	}

	if (unwrapped.refresh_token) {
		setCookie(event, 'refresh_token', unwrapped.refresh_token, {
			...baseOptions,
			httpOnly: true,
			maxAge: SEVEN_DAYS_SECONDS
		})
	}

	if (unwrapped.id_token) {
		setCookie(event, 'id_token', unwrapped.id_token, {
			...baseOptions,
			httpOnly: true,
			maxAge: ONE_DAY_SECONDS
		})
	}

	setCookie(event, 'auth_provider', provider, {
		...baseOptions,
		maxAge: SEVEN_DAYS_SECONDS
	})

	setCookie(event, 'is_authenticated', 'true', {
		...baseOptions,
		maxAge: ONE_DAY_SECONDS
	})

	if (activeClusterUrl) {
		setCookie(event, 'active_cluster_url', activeClusterUrl, {
			...baseOptions,
			maxAge: SEVEN_DAYS_SECONDS
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
	const isSecure = isRequestSecure(event)
	setCookie(event, 'user_clusters', JSON.stringify(clusters || []), {
		path: '/',
		sameSite: 'lax',
		secure: isSecure,
		httpOnly: false,
		maxAge: SEVEN_DAYS_SECONDS
	})
}

const KNOWN_AUTH_COOKIES = [
	'access_token',
	'access_token_exp',
	'auth_refreshed_at',
	'refresh_token',
	'id_token',
	'auth_provider',
	'is_authenticated',
	'active_cluster_url',
	'user_clusters'
]

export function clearAuthCookies(event: H3Event) {
	const paths = ['/', '/auth', '/api']
	const dynamicSessionCookies = Object.keys(parseCookies(event)).filter(
		(name) => name.startsWith('patrolli_') || name.startsWith('auth_')
	)
	const cookieNames = new Set([...KNOWN_AUTH_COOKIES, ...dynamicSessionCookies])

	for (const name of cookieNames) {
		for (const p of paths) {
			deleteCookie(event, name, { path: p })
		}
	}
}

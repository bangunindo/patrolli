// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { H3Event } from 'h3'
import {
	clearAuthCookies,
	extractTokenPayload,
	getAuthCookies,
	type IAuthTokens,
	isJwtExpired,
	setAuthCookies
} from '@@/server/utils/auth-cookies'
import {
	forwardToCluster,
	getClustersConfig,
	resolveClusterTargets
} from '@@/server/services/cluster.service'
import { refreshKeycloakToken } from '@@/server/modules/keycloak/keycloak.api'
import {
	formatKeycloakConnectionError,
	getValidatedKeycloakConfig
} from '@@/server/modules/keycloak/keycloak.utils'
import type { AuthProvider } from '@@/shared/types/auth.type'

export interface IRefreshParams {
	refreshToken: string
	authProvider: AuthProvider
	clusterHeader?: string | null
	reqHeaders?: Record<string, any>
	query?: Record<string, any>
}

export interface IRefreshResult {
	ok: boolean
	status: number
	isSessionInvalid: boolean
	tokens?: IAuthTokens
	resolvedClusterUrl?: string | null
	errorPayload?: any
}

export function sanitizeUnauthenticatedHeaders(
	headers?: Record<string, any>
): Record<string, any> | undefined {
	if (!headers) return undefined
	const sanitized: Record<string, any> = {}
	for (const [key, value] of Object.entries(headers)) {
		const lower = key.toLowerCase()
		if (lower !== 'authorization' && lower !== 'cookie') {
			sanitized[key] = value
		}
	}
	return sanitized
}

async function executeRefresh(
	event: H3Event | undefined,
	params: IRefreshParams
): Promise<IRefreshResult> {
	const { refreshToken, authProvider, clusterHeader, reqHeaders, query } = params

	if (authProvider === 'keycloak') {
		const { config: kcConfig, errorPayload } = getValidatedKeycloakConfig()
		if (!kcConfig) {
			return {
				ok: false,
				status: 500,
				isSessionInvalid: false,
				errorPayload
			}
		}

		try {
			const response = await refreshKeycloakToken(kcConfig, String(refreshToken))
			const data = response.data

			if (response.status >= 400) {
				const isSessionInvalid =
					response.status === 401 ||
					response.status === 403 ||
					data?.error === 'invalid_grant'

				return {
					ok: false,
					status: response.status,
					isSessionInvalid,
					errorPayload: {
						status_code: response.status,
						success: false,
						is_session_invalid: isSessionInvalid,
						error: {
							title: 'Keycloak Refresh Error',
							detail:
								data?.error_description ||
								data?.error ||
								'Failed to refresh token with Keycloak'
						},
						data
					}
				}
			}

			const tokens = extractTokenPayload(data)
			return {
				ok: true,
				status: 200,
				isSessionInvalid: false,
				tokens
			}
		} catch (error: any) {
			return {
				ok: false,
				status: 502,
				isSessionInvalid: false,
				errorPayload: {
					status_code: 502,
					success: false,
					error: {
						title: 'Bad Gateway',
						detail: formatKeycloakConnectionError(error)
					}
				}
			}
		}
	}

	const allClusters = getClustersConfig(event)
	const targetClusters = resolveClusterTargets(clusterHeader, allClusters)

	if (targetClusters.length === 0) {
		return {
			ok: false,
			status: 502,
			isSessionInvalid: false,
			errorPayload: {
				status_code: 502,
				success: false,
				error: {
					title: 'Bad Gateway',
					detail: 'No clusters configured for refresh token exchange'
				}
			}
		}
	}

	const refreshPayload = { refresh_token: refreshToken }
	const cleanHeaders = sanitizeUnauthenticatedHeaders(reqHeaders)

	const responses = await Promise.all(
		targetClusters.map(async (cluster) => {
			const res = await forwardToCluster(cluster, '/api/v1/auth/refresh', {
				method: 'POST',
				headers: cleanHeaders,
				query,
				body: refreshPayload
			})
			return { cluster, data: res.data, status: res.status }
		})
	)

	const successfulResponse = responses.find((r) => r.status >= 200 && r.status < 300)

	if (successfulResponse) {
		const tokens = extractTokenPayload(successfulResponse.data)
		return {
			ok: true,
			status: successfulResponse.status,
			isSessionInvalid: false,
			tokens,
			resolvedClusterUrl: clusterHeader || successfulResponse.cluster.clientPath
		}
	}

	const isSessionInvalid =
		responses.length > 0 && responses.every((r) => r.status === 401 || r.status === 403)
	const primaryError = responses.find((r) => r.status !== 401 && r.status !== 403) ?? responses[0]
	const errorStatus = primaryError?.status || 502
	return {
		ok: false,
		status: errorStatus,
		isSessionInvalid,
		errorPayload: primaryError?.data || {
			status_code: errorStatus,
			success: false,
			error: {
				title: 'Refresh Failed',
				detail: 'Failed to refresh access token'
			}
		}
	}
}

/**
 * Performs a stateless token refresh and applies updated Set-Cookie headers to the event.
 */
export async function performTokenRefresh(
	event: H3Event | undefined,
	params: IRefreshParams
): Promise<IRefreshResult> {
	const result = await executeRefresh(event, params)

	if (event) {
		if (result.ok && result.tokens) {
			const activeClusterUrl =
				params.authProvider === 'local'
					? (result.resolvedClusterUrl ?? params.clusterHeader)
					: undefined
			setAuthCookies(event, result.tokens, params.authProvider, activeClusterUrl)
			event.context ??= {}
			if (result.tokens.access_token) {
				event.context._refreshedAccessToken = result.tokens.access_token
			}
			if (result.tokens.id_token) {
				event.context._refreshedIdToken = result.tokens.id_token
			}
		} else if (result.isSessionInvalid) {
			clearAuthCookies(event)
		}
	}

	return result
}

/**
 * Attempts a transparent token refresh using HttpOnly cookies when an upstream request returns 401.
 * Returns the new access token if successful, or null otherwise.
 */
export async function tryTransparentRefreshOn401(
	event: H3Event,
	clusterHeader?: string | null
): Promise<string | null> {
	const { refresh_token, auth_provider, active_cluster_url } = getAuthCookies(event)
	if (!refresh_token) return null

	const result = await performTokenRefresh(event, {
		refreshToken: refresh_token,
		authProvider: auth_provider || 'local',
		clusterHeader: clusterHeader ?? active_cluster_url
	})

	if (result.ok && result.tokens?.access_token) {
		return result.tokens.access_token
	}
	return null
}

function extractBearerToken(headers?: Record<string, any>): string | undefined {
	const raw = headers?.authorization || headers?.Authorization
	if (typeof raw !== 'string' || !raw.trim()) return undefined
	return raw.startsWith('Bearer ') ? raw.slice(7).trim() : raw.trim()
}

export function withBearerAuthorization(
	headers: Record<string, any>,
	token: string
): Record<string, any> {
	const { authorization: _lower, Authorization: _upper, ...rest } = headers
	return { ...rest, authorization: `Bearer ${token}` }
}

/**
 * Proactively refreshes an expired (or missing) access token before fanning out requests to clusters.
 */
export async function resolveEffectiveRequestHeaders(
	event: H3Event,
	reqHeaders: Record<string, any>,
	clusterHeader?: string | null
): Promise<Record<string, any>> {
	const existingBearer = extractBearerToken(reqHeaders)
	if (existingBearer && !isJwtExpired(existingBearer)) {
		return reqHeaders
	}

	if (event.context?._refreshedAccessToken) {
		return withBearerAuthorization(reqHeaders, event.context._refreshedAccessToken)
	}

	const { access_token, refresh_token } = getAuthCookies(event)
	if (refresh_token && (!access_token || isJwtExpired(access_token))) {
		const newToken = await tryTransparentRefreshOn401(event, clusterHeader)
		if (newToken) {
			return withBearerAuthorization(reqHeaders, newToken)
		}
	}
	return reqHeaders
}

/**
 * Executes a single-item cluster request with pre-emptive expiry refresh and a single reactive 401 retry.
 */
export async function executeSingleClusterRequestWithAuthRetry<
	TItem,
	TResult extends { status: number }
>(
	event: H3Event,
	options: {
		item: TItem
		reqHeaders: Record<string, any>
		clusterHeader?: string | null
		execute: (item: TItem, headers: Record<string, any>) => Promise<TResult>
	}
): Promise<{ result: TResult; effectiveHeaders: Record<string, any> }> {
	const { results, effectiveHeaders } = await executeClusterRequestsWithAuthRetry(event, {
		items: [options.item],
		reqHeaders: options.reqHeaders,
		clusterHeader: options.clusterHeader,
		execute: options.execute
	})
	return { result: results[0]!, effectiveHeaders }
}

/**
 * Executes per-item cluster requests with pre-emptive expiry refresh and a single reactive 401 retry.
 */
export async function executeClusterRequestsWithAuthRetry<
	TItem,
	TResult extends { status: number }
>(
	event: H3Event,
	options: {
		items: TItem[]
		reqHeaders: Record<string, any>
		clusterHeader?: string | null
		execute: (item: TItem, headers: Record<string, any>) => Promise<TResult>
	}
): Promise<{ results: TResult[]; effectiveHeaders: Record<string, any> }> {
	let effectiveHeaders = await resolveEffectiveRequestHeaders(
		event,
		options.reqHeaders,
		options.clusterHeader
	)

	let results = await Promise.all(
		options.items.map((item) => options.execute(item, effectiveHeaders))
	)

	if (results.some((r) => r.status === 401) && !event.context?._refreshedAccessToken) {
		const newToken = await tryTransparentRefreshOn401(event, options.clusterHeader)
		if (newToken) {
			effectiveHeaders = withBearerAuthorization(options.reqHeaders, newToken)
			results = await Promise.all(
				results.map((prev, index) => {
					if (prev.status !== 401) return prev
					return options.execute(options.items[index]!, effectiveHeaders)
				})
			)
		}
	}

	return { results, effectiveHeaders }
}

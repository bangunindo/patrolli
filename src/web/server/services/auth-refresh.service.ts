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
import { refreshOidcTokens } from '@@/server/modules/oidc/oidc.service'
import {
	formatOidcConnectionError,
	getValidatedOidcConfig,
	selectUpstreamAuthTokens
} from '@@/server/modules/oidc/oidc.utils'
import type { AuthProvider } from '@@/shared/types/auth.type'

interface IRefreshParams {
	refreshToken: string
	authProvider: AuthProvider
	clusterHeader?: string | null
	reqHeaders?: Record<string, any>
	query?: Record<string, any>
}

interface IRefreshResult {
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

	if (authProvider === 'oidc') {
		const { config: oidcConfig, errorPayload } = getValidatedOidcConfig()
		if (!oidcConfig) {
			return {
				ok: false,
				status: 500,
				isSessionInvalid: false,
				errorPayload
			}
		}

		try {
			const tokenSet = await refreshOidcTokens(oidcConfig, String(refreshToken))
			const tokens = selectUpstreamAuthTokens(tokenSet, oidcConfig.token_source)
			if (!tokens.access_token) {
				const detail =
					oidcConfig.token_source === 'id_token'
						? 'OIDC provider did not return an id_token or JWT access_token during refresh; configure the provider to return id_token on refresh or use token_source: access_token'
						: 'OIDC provider did not return a usable token during refresh'
				return {
					ok: false,
					status: 401,
					isSessionInvalid: true,
					errorPayload: {
						status_code: 401,
						success: false,
						is_session_invalid: true,
						error: {
							title: 'OIDC Refresh Error',
							detail
						}
					}
				}
			}
			return {
				ok: true,
				status: 200,
				isSessionInvalid: false,
				tokens
			}
		} catch (error: any) {
			const oauthError = error?.error
			const responseStatus =
				typeof error?.status === 'number' && error.status >= 400 && error.status < 600
					? error.status
					: undefined
			const isOAuthBodyError =
				error?.code === 'OAUTH_RESPONSE_BODY_ERROR' || Boolean(oauthError)
			const status = responseStatus ?? (isOAuthBodyError ? 400 : 502)
			const isSessionInvalid =
				status === 401 || status === 403 || oauthError === 'invalid_grant'

			return {
				ok: false,
				status,
				isSessionInvalid,
				errorPayload: {
					status_code: status,
					success: false,
					is_session_invalid: isSessionInvalid,
					error: {
						title: isOAuthBodyError ? 'OIDC Refresh Error' : 'Bad Gateway',
						detail: formatOidcConnectionError(
							error,
							'Failed to refresh token with OIDC provider'
						)
					},
					...(oauthError
						? {
								data: {
									error: oauthError,
									error_description: error?.error_description
								}
							}
						: {})
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

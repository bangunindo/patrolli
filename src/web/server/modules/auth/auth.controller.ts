// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { H3Event } from 'h3'
import {
	appendResponseHeader,
	getQuery,
	getRequestHeaders,
	getRequestHost,
	getRequestProtocol,
	readBody,
	setResponseHeader,
	setResponseHeaders,
	setResponseStatus
} from 'h3'
import { clearAuthCookies, setAuthCookies } from '@@/server/utils/auth-cookies'
import { renderOidcErrorHtml, renderOidcSuccessHtml } from '@@/server/utils/auth-html'
import {
	getClustersConfig,
	getKeycloakConfig,
	resolveClusterTargets
} from '@@/server/services/cluster.service'
import { exchangeKeycloakCode, refreshKeycloakToken } from '@@/server/modules/keycloak/keycloak.api'
import { verifyUserClusterAccess } from '@@/server/services/cluster-access.service'
import { forwardAuthToCluster } from './auth.api'
import { extractAuthRefreshParams, isLogoutPath } from './auth.utils'

/**
 * Handles /auth/[...path] multi-cluster authentication route (login, logout, etc.)
 */
export async function handleAuthProxy(event: H3Event) {
	const method = event.method
	const pathParam = event.context.params?.path || ''
	if (pathParam.startsWith('oidc')) {
		return
	}
	const fullAuthPath = `/auth/${pathParam}`
	const isLogout = isLogoutPath(pathParam, fullAuthPath)

	const allClusters = getClustersConfig(event)
	const reqHeaders = getRequestHeaders(event)
	const query = getQuery(event)
	const clusterHeader =
		(reqHeaders['x-cluster-url'] as string | undefined) ||
		(query.cluster as string | undefined) ||
		null

	const targetClusters = resolveClusterTargets(clusterHeader, allClusters)

	if (targetClusters.length === 0) {
		if (isLogout) {
			clearAuthCookies(event)
			setResponseStatus(event, 200)
			return {
				status_code: 200,
				success: true,
				data: { message: 'Logout successful' }
			}
		}
		const errorPayload = {
			status_code: 502,
			success: false,
			error: {
				title: 'Bad Gateway',
				detail: 'No clusters configured. Please configure clusters in config-fe.yaml or environment variables.'
			}
		}
		setResponseStatus(event, 502)
		return errorPayload
	}

	let body: any
	if (method !== 'GET' && method !== 'HEAD') {
		try {
			body = await readBody(event)
		} catch {
			body = undefined
		}
	}

	// 1. Single target cluster
	const singleTarget = targetClusters[0]
	if (targetClusters.length === 1 && singleTarget) {
		const res = await forwardAuthToCluster(singleTarget, fullAuthPath, {
			method,
			headers: reqHeaders,
			query,
			body
		})

		const responseHeaders: Record<string, string> = {}
		res.headers.forEach((value, key) => {
			const lowerKey = key.toLowerCase()
			if (lowerKey === 'set-cookie') {
				appendResponseHeader(event, 'set-cookie', value)
			} else if (lowerKey !== 'content-encoding' && lowerKey !== 'content-length') {
				responseHeaders[key] = value
			}
		})
		setResponseHeaders(event, responseHeaders)

		if (isLogout) {
			clearAuthCookies(event)
			setResponseStatus(event, 200)
			return {
				status_code: 200,
				success: true,
				data:
					res.data && res.status >= 200 && res.status < 300
						? res.data
						: { message: 'Logout successful' }
			}
		} else if (res.status === 200 && res.data) {
			setAuthCookies(event, res.data, 'local')
			const token =
				res.data?.access_token ||
				res.data?.data?.access_token ||
				res.data?.token ||
				res.data?.data?.token
			if (token) {
				await verifyUserClusterAccess(event, token)
			}
		}

		setResponseStatus(event, res.status)
		return res.data
	}

	// 2. Multi-cluster fan-out (login / logout to all clusters)
	const responses = await Promise.all(
		targetClusters.map(async (cluster) => {
			const res = await forwardAuthToCluster(cluster, fullAuthPath, {
				method,
				headers: reqHeaders,
				query,
				body
			})
			return { cluster, data: res.data, status: res.status, headers: res.headers }
		})
	)

	// Collect and set all cookies from all cluster responses
	for (const r of responses) {
		if (r.headers) {
			r.headers.forEach((value, key) => {
				if (key.toLowerCase() === 'set-cookie') {
					appendResponseHeader(event, 'set-cookie', value)
				}
			})
		}
	}

	if (isLogout) {
		clearAuthCookies(event)
		setResponseStatus(event, 200)
		const successfulLogout = responses.find((r) => r.status >= 200 && r.status < 300)
		return {
			status_code: 200,
			success: true,
			data: successfulLogout?.data || { message: 'Logout successful' }
		}
	}

	const successfulResponse = responses.find((r) => r.status >= 200 && r.status < 300)

	if (successfulResponse) {
		if (successfulResponse.data) {
			setAuthCookies(event, successfulResponse.data, 'local')
			const token =
				successfulResponse.data?.access_token ||
				successfulResponse.data?.data?.access_token ||
				successfulResponse.data?.token ||
				successfulResponse.data?.data?.token
			if (token) {
				await verifyUserClusterAccess(event, token)
			}
		}
		setResponseStatus(event, successfulResponse.status)
		return successfulResponse.data
	}

	const firstError = responses[0]
	if (firstError) {
		setResponseStatus(event, firstError.status)
		return firstError.data
	}
}

/**
 * Handles /api/auth/refresh endpoint for local & keycloak token refreshing
 */
export async function handleAuthRefresh(event: H3Event) {
	let body: any = {}
	try {
		body = (await readBody(event)) || {}
	} catch {
		body = {}
	}

	const { refreshToken, authProvider, clusterHeader, reqHeaders, query } =
		extractAuthRefreshParams(event, body)

	if (!refreshToken) {
		const errorPayload = {
			status_code: 401,
			success: false,
			error: {
				title: 'Unauthorized',
				detail: 'Refresh token not found in cookies or request'
			}
		}
		clearAuthCookies(event)
		setResponseStatus(event, 401)
		return errorPayload
	}

	// 1. Keycloak Refresh Flow
	if (authProvider === 'keycloak') {
		const kcConfig = getKeycloakConfig()
		if (!kcConfig || !kcConfig.host || !kcConfig.client_id) {
			const errorPayload = {
				status_code: 500,
				success: false,
				error: {
					title: 'Configuration Error',
					detail: 'Keycloak configuration is missing or incomplete in config-fe.yaml'
				}
			}
			setResponseStatus(event, 500)
			return errorPayload
		}

		try {
			const response = await refreshKeycloakToken(kcConfig, String(refreshToken))
			const data = response.data

			if (response.status >= 400) {
				const errorPayload = {
					status_code: response.status,
					success: false,
					error: {
						title: 'Keycloak Refresh Error',
						detail:
							data?.error_description ||
							data?.error ||
							'Failed to refresh token with Keycloak'
					},
					data
				}
				setResponseStatus(event, response.status)
				return errorPayload
			}

			setAuthCookies(event, data, 'keycloak')

			return {
				status_code: 200,
				success: true,
				data
			}
		} catch (error: any) {
			const causeDetail = error?.cause?.message || error?.cause?.code || error?.cause
			const errMessage = error?.message || 'Failed to connect to Keycloak token endpoint'
			const fullDetail = causeDetail ? `${errMessage} (${causeDetail})` : errMessage

			const errorPayload = {
				status_code: 502,
				success: false,
				error: {
					title: 'Bad Gateway',
					detail: fullDetail
				}
			}
			setResponseStatus(event, 502)
			return errorPayload
		}
	}

	// 2. Local Cluster Refresh Flow
	const allClusters = getClustersConfig(event)
	const targetClusters = resolveClusterTargets(clusterHeader, allClusters)

	if (targetClusters.length === 0) {
		const errorPayload = {
			status_code: 502,
			success: false,
			error: {
				title: 'Bad Gateway',
				detail: 'No clusters configured for refresh token exchange'
			}
		}
		setResponseStatus(event, 502)
		return errorPayload
	}

	const refreshPayload = {
		refresh_token: refreshToken
	}

	const responses = await Promise.all(
		targetClusters.map(async (cluster) => {
			const res = await forwardAuthToCluster(cluster, '/auth/refresh', {
				method: 'POST',
				headers: reqHeaders,
				query,
				body: refreshPayload
			})
			return { cluster, data: res.data, status: res.status, headers: res.headers }
		})
	)

	for (const r of responses) {
		if (r.headers) {
			r.headers.forEach((value, key) => {
				if (key.toLowerCase() === 'set-cookie') {
					appendResponseHeader(event, 'set-cookie', value)
				}
			})
		}
	}

	const successfulResponse = responses.find((r) => r.status >= 200 && r.status < 300)

	if (successfulResponse) {
		if (successfulResponse.data) {
			setAuthCookies(event, successfulResponse.data, 'local')
		}
		setResponseStatus(event, successfulResponse.status)
		return successfulResponse.data
	}

	const firstError = responses[0]
	if (firstError) {
		if (firstError.status === 401 || firstError.status === 403) {
			clearAuthCookies(event)
		}
		setResponseStatus(event, firstError.status)
		return firstError.data
	}
}

/**
 * Handles /auth/oidc/callback route rendering HTML popup result
 */
export async function handleOidcCallback(event: H3Event) {
	const query = getQuery(event)
	const code = query.code as string | undefined
	const error = (query.error_description || query.error) as string | undefined

	if (error || !code) {
		const errMessage = error || 'Authorization code not found in callback query.'
		setResponseStatus(event, 400)
		setResponseHeader(event, 'content-type', 'text/html; charset=utf-8')
		return renderOidcErrorHtml(errMessage)
	}

	const kcConfig = getKeycloakConfig()
	if (!kcConfig || !kcConfig.host || !kcConfig.client_id) {
		const errMessage = 'Keycloak configuration is missing or incomplete in config-fe.yaml'
		setResponseStatus(event, 500)
		setResponseHeader(event, 'content-type', 'text/html; charset=utf-8')
		return renderOidcErrorHtml(errMessage)
	}

	const host = getRequestHost(event, { xForwardedHost: true })
	const proto = getRequestProtocol(event, { xForwardedProto: true })
	const dynamicRedirectUri = host
		? `${proto}://${host}/auth/oidc/callback`
		: kcConfig.redirect_uri

	try {
		const response = await exchangeKeycloakCode(kcConfig, code, dynamicRedirectUri)
		const data = response.data

		if (response.status >= 400 || !data?.access_token) {
			const errMessage =
				data?.error_description ||
				data?.error ||
				`Token exchange failed with status ${response.status}`
			setResponseStatus(event, response.status >= 400 ? response.status : 400)
			setResponseHeader(event, 'content-type', 'text/html; charset=utf-8')
			return renderOidcErrorHtml(errMessage)
		}

		setAuthCookies(event, data, 'keycloak')
		setResponseHeader(event, 'content-type', 'text/html; charset=utf-8')
		return renderOidcSuccessHtml(data.access_token, data.refresh_token, data.id_token)
	} catch (err: any) {
		const causeDetail = err?.cause?.message || err?.cause?.code || err?.cause
		const errMessage = err?.message || 'Failed to connect to Keycloak token endpoint'
		const fullDetail = causeDetail ? `${errMessage} (${causeDetail})` : errMessage

		setResponseStatus(event, 502)
		setResponseHeader(event, 'content-type', 'text/html; charset=utf-8')
		return renderOidcErrorHtml(fullDetail)
	}
}

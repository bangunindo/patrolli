// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { H3Event } from 'h3'
import {
	appendResponseHeader,
	getQuery,
	getRequestHeaders,
	readBody,
	setResponseHeaders,
	setResponseStatus
} from 'h3'
import {
	buildAuthSessionPayload,
	clearAuthCookies,
	extractTokenPayload,
	setAuthCookies
} from '@@/server/utils/auth-cookies'
import { getClustersConfig, resolveClusterTargets } from '@@/server/services/cluster.service'
import {
	executeClusterRequestsWithAuthRetry,
	performTokenRefresh,
	sanitizeUnauthenticatedHeaders
} from '@@/server/services/auth-refresh.service'
import { verifyUserClusterAccess } from '@@/server/services/cluster-access.service'
import type { AuthProvider } from '@@/shared/types/auth.type'
import type { ICluster } from '@@/shared/types/cluster.type'
import { forwardAuthToCluster } from './auth.api'
import { extractAuthRefreshParams, isLoginPath, isLogoutPath } from './auth.utils'

async function forwardAuthToTargets(
	event: H3Event,
	targetClusters: ICluster[],
	fullAuthPath: string,
	options: {
		isAuthenticatedAction: boolean
		method: string
		reqHeaders: Record<string, any>
		clusterHeader: string | null
		query: Record<string, any>
		body: any
	}
) {
	const executeItem = async (cluster: ICluster, headers: Record<string, any>) => {
		const res = await forwardAuthToCluster(cluster, fullAuthPath, {
			method: options.method,
			headers,
			query: options.query,
			body: options.body
		})
		return { cluster, data: res.data, status: res.status, headers: res.headers }
	}

	if (options.isAuthenticatedAction) {
		const { results } = await executeClusterRequestsWithAuthRetry(event, {
			items: targetClusters,
			reqHeaders: options.reqHeaders,
			clusterHeader: options.clusterHeader,
			execute: executeItem
		})
		return results
	}

	return Promise.all(targetClusters.map((cluster) => executeItem(cluster, options.reqHeaders)))
}

export async function establishAuthSession(
	event: H3Event,
	rawData: any,
	provider: AuthProvider,
	activeClusterUrl?: string | null
) {
	const tokens = extractTokenPayload(rawData)
	setAuthCookies(event, tokens, provider, activeClusterUrl)
	if (tokens.access_token) {
		await verifyUserClusterAccess(event, tokens.access_token)
	}
	return buildAuthSessionPayload(tokens)
}

export async function handleAuthProxy(event: H3Event) {
	const method = event.method
	const pathParam = event.context.params?.path || ''
	if (pathParam.startsWith('oidc')) {
		return
	}
	const cleanPathParam = pathParam.replace(/^(?:\/?v1\/)?(?:\/?auth\/)?/, '')
	const fullAuthPath = `/api/v1/auth/${cleanPathParam}`
	const isLogin = isLoginPath(pathParam, fullAuthPath)
	const isLogout = isLogoutPath(pathParam, fullAuthPath)
	const isAuthenticatedAction = !isLogin && !isLogout

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

	const effectiveHeaders = isLogin ? sanitizeUnauthenticatedHeaders(reqHeaders) || {} : reqHeaders

	const responses = await forwardAuthToTargets(event, targetClusters, fullAuthPath, {
		isAuthenticatedAction,
		method,
		reqHeaders: effectiveHeaders,
		clusterHeader,
		query,
		body
	})

	if (targetClusters.length === 1 && responses[0]?.headers) {
		const responseHeaders: Record<string, string> = {}
		responses[0].headers.forEach((value, key) => {
			const lowerKey = key.toLowerCase()
			if (
				lowerKey !== 'set-cookie' &&
				lowerKey !== 'content-encoding' &&
				lowerKey !== 'content-length'
			) {
				responseHeaders[key] = value
			}
		})
		setResponseHeaders(event, responseHeaders)
	}

	for (const r of responses) {
		r.headers?.forEach((value, key) => {
			if (key.toLowerCase() === 'set-cookie') {
				appendResponseHeader(event, 'set-cookie', value)
			}
		})
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
		setResponseStatus(event, successfulResponse.status)
		const tokens = extractTokenPayload(successfulResponse.data)
		if (tokens.access_token || tokens.refresh_token) {
			const sessionData = await establishAuthSession(
				event,
				tokens,
				'local',
				clusterHeader || successfulResponse.cluster.clientPath
			)
			return {
				status_code: successfulResponse.status,
				success: true,
				data: sessionData
			}
		}
		return successfulResponse.data
	}

	const firstError = responses[0]
	if (firstError) {
		setResponseStatus(event, firstError.status)
		return firstError.data
	}
}

export async function handleAuthRefresh(event: H3Event, forcedProvider?: AuthProvider) {
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
			is_session_invalid: true,
			error: {
				title: 'Unauthorized',
				detail: 'Refresh token not found in cookies or request'
			}
		}
		clearAuthCookies(event)
		setResponseStatus(event, 401)
		return errorPayload
	}

	const result = await performTokenRefresh(event, {
		refreshToken: String(refreshToken),
		authProvider: forcedProvider ?? authProvider,
		clusterHeader,
		reqHeaders,
		query
	})

	setResponseStatus(event, result.status)
	if (!result.ok) {
		return result.errorPayload
	}

	return {
		status_code: result.status,
		success: true,
		data: buildAuthSessionPayload(result.tokens)
	}
}

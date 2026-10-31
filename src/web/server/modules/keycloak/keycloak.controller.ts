// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { H3Event } from 'h3'
import { getRequestHost, getRequestProtocol, readBody, setResponseStatus } from 'h3'
import { setAuthCookies } from '@@/server/utils/auth-cookies'
import { verifyUserClusterAccess } from '@@/server/services/cluster-access.service'
import { getKeycloakConfig } from '@@/server/services/cluster.service'
import { exchangeKeycloakCode, refreshKeycloakToken } from './keycloak.api'
import { buildKeycloakAuthUrl } from './keycloak.utils'

/**
 * Handles Keycloak login URL generation (GET /api/auth/keycloak/login)
 */
export function handleKeycloakLogin(event: H3Event) {
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

	const host = getRequestHost(event, { xForwardedHost: true })
	const proto = getRequestProtocol(event, { xForwardedProto: true })
	const dynamicRedirectUri = host
		? `${proto}://${host}/auth/oidc/callback`
		: kcConfig.redirect_uri

	const authUrl = buildKeycloakAuthUrl(kcConfig, dynamicRedirectUri)

	return {
		status_code: 200,
		success: true,
		data: {
			url: authUrl,
			host: kcConfig.host,
			client_id: kcConfig.client_id,
			redirect_uri: dynamicRedirectUri
		}
	}
}

/**
 * Handles Keycloak authorization code callback (POST /api/auth/keycloak/callback)
 */
export async function handleKeycloakCallback(event: H3Event) {
	const body = await readBody(event).catch(() => ({}))
	const code = body?.code
	const host = getRequestHost(event, { xForwardedHost: true })
	const proto = getRequestProtocol(event, { xForwardedProto: true })
	const defaultDynamicRedirect = host ? `${proto}://${host}/auth/oidc/callback` : undefined
	const redirectUri =
		body?.redirect_uri || defaultDynamicRedirect || 'https://localhost:3000/auth/oidc/callback'

	if (!code) {
		const errorPayload = {
			status_code: 400,
			success: false,
			error: {
				title: 'Bad Request',
				detail: 'Authorization code is required'
			}
		}
		setResponseStatus(event, 400)
		return errorPayload
	}

	const kcConfig = getKeycloakConfig()
	if (!kcConfig || !kcConfig.host || !kcConfig.client_id) {
		const errorPayload = {
			status_code: 500,
			success: false,
			error: {
				title: 'Configuration Error',
				detail: 'Keycloak configuration is missing in config-fe.yaml'
			}
		}
		setResponseStatus(event, 500)
		return errorPayload
	}

	try {
		const response = await exchangeKeycloakCode(kcConfig, code, redirectUri)
		const data = response.data

		if (response.status >= 400) {
			const errorPayload = {
				status_code: response.status,
				success: false,
				error: {
					title: 'Keycloak Authentication Error',
					detail:
						data?.error_description ||
						data?.error ||
						'Failed to exchange token with Keycloak'
				},
				data
			}
			setResponseStatus(event, response.status)
			return errorPayload
		}

		setAuthCookies(event, data, 'keycloak')
		if (data?.access_token) {
			await verifyUserClusterAccess(event, data.access_token)
		}

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

/**
 * Handles Keycloak refresh token endpoint (POST /api/auth/keycloak/refresh)
 */
export async function handleKeycloakRefresh(event: H3Event) {
	const body = await readBody(event).catch(() => ({}))
	const refreshToken = body?.refresh_token

	if (!refreshToken) {
		const errorPayload = {
			status_code: 400,
			success: false,
			error: {
				title: 'Bad Request',
				detail: 'Refresh token is required'
			}
		}
		setResponseStatus(event, 400)
		return errorPayload
	}

	const kcConfig = getKeycloakConfig()
	if (!kcConfig || !kcConfig.host || !kcConfig.client_id) {
		const errorPayload = {
			status_code: 500,
			success: false,
			error: {
				title: 'Configuration Error',
				detail: 'Keycloak configuration is missing in config-fe.yaml'
			}
		}
		setResponseStatus(event, 500)
		return errorPayload
	}

	try {
		const response = await refreshKeycloakToken(kcConfig, refreshToken)
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

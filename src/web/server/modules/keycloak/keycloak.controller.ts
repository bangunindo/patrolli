// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { H3Event } from 'h3'
import { readBody, setResponseStatus } from 'h3'
import { establishAuthSession, handleAuthRefresh } from '@@/server/modules/auth/auth.controller'
import { exchangeKeycloakCode } from './keycloak.api'
import {
	buildKeycloakAuthUrl,
	formatKeycloakConnectionError,
	getValidatedKeycloakConfig,
	resolveDynamicOidcRedirectUri
} from './keycloak.utils'

/**
 * Handles Keycloak login URL generation (GET /api/auth/keycloak/login)
 */
export function handleKeycloakLogin(event: H3Event) {
	const { config: kcConfig, errorPayload } = getValidatedKeycloakConfig()
	if (!kcConfig) {
		setResponseStatus(event, 500)
		return errorPayload
	}

	const dynamicRedirectUri = resolveDynamicOidcRedirectUri(event, kcConfig.redirect_uri)
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
	const redirectUri =
		body?.redirect_uri ||
		resolveDynamicOidcRedirectUri(event) ||
		'https://localhost:3000/auth/oidc/callback'

	if (!code) {
		setResponseStatus(event, 400)
		return {
			status_code: 400,
			success: false,
			error: {
				title: 'Bad Request',
				detail: 'Authorization code is required'
			}
		}
	}

	const { config: kcConfig, errorPayload } = getValidatedKeycloakConfig()
	if (!kcConfig) {
		setResponseStatus(event, 500)
		return errorPayload
	}

	try {
		const response = await exchangeKeycloakCode(kcConfig, code, redirectUri)
		const data = response.data

		if (response.status >= 400) {
			setResponseStatus(event, response.status)
			return {
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
		}

		const sessionData = await establishAuthSession(event, data, 'keycloak')

		return {
			status_code: 200,
			success: true,
			data: sessionData
		}
	} catch (error: any) {
		setResponseStatus(event, 502)
		return {
			status_code: 502,
			success: false,
			error: {
				title: 'Bad Gateway',
				detail: formatKeycloakConnectionError(error)
			}
		}
	}
}

/**
 * Handles Keycloak refresh token endpoint (POST /api/auth/keycloak/refresh)
 */
export async function handleKeycloakRefresh(event: H3Event) {
	return handleAuthRefresh(event, 'keycloak')
}

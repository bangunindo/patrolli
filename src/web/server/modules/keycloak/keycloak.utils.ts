// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { getRequestHost, getRequestProtocol, type H3Event } from 'h3'
import { getKeycloakConfig } from '@@/server/services/cluster.service'
import type { IKeycloakConfig } from '@@/shared/types/cluster.type'

/**
 * Validates and returns the Keycloak configuration, or a standardized 500 error payload.
 */
export function getValidatedKeycloakConfig():
	| { config: IKeycloakConfig; errorPayload?: undefined }
	| { config?: undefined; errorPayload: Record<string, any> } {
	const kcConfig = getKeycloakConfig()
	if (!kcConfig || !kcConfig.host || !kcConfig.client_id) {
		return {
			errorPayload: {
				status_code: 500,
				success: false,
				error: {
					title: 'Configuration Error',
					detail: 'Keycloak configuration is missing or incomplete in config-fe.yaml'
				}
			}
		}
	}
	return { config: kcConfig }
}

/**
 * Resolves the dynamic OIDC callback redirect URI from the incoming request headers.
 */
export function resolveDynamicOidcRedirectUri(
	event: H3Event,
	fallbackUri?: string
): string | undefined {
	const host = getRequestHost(event, { xForwardedHost: true })
	const proto = getRequestProtocol(event, { xForwardedProto: true })
	return host ? `${proto}://${host}/auth/oidc/callback` : fallbackUri
}

/**
 * Formats upstream Keycloak connection errors including underlying cause details.
 */
export function formatKeycloakConnectionError(
	error: any,
	defaultMessage = 'Failed to connect to Keycloak token endpoint'
): string {
	const causeDetail = error?.cause?.message || error?.cause?.code || error?.cause
	const errMessage = error?.message || defaultMessage
	return causeDetail ? `${errMessage} (${causeDetail})` : errMessage
}

/**
 * Builds the Keycloak authorization URL for initiating OIDC flow
 */
export function buildKeycloakAuthUrl(config: IKeycloakConfig, redirectUri?: string): string {
	const effectiveRedirectUri = encodeURIComponent(redirectUri || config.redirect_uri)
	return `${config.host}/protocol/openid-connect/auth?client_id=${config.client_id}&redirect_uri=${effectiveRedirectUri}&response_type=code&scope=openid`
}

/**
 * Builds the URL-encoded payload for authorization code exchange
 */
export function buildTokenExchangeBody(
	config: IKeycloakConfig,
	code: string,
	redirectUri?: string
): string {
	const params = new URLSearchParams({
		grant_type: 'authorization_code',
		client_id: config.client_id,
		code: String(code),
		redirect_uri: redirectUri || config.redirect_uri
	})

	if (config.client_secret) {
		params.set('client_secret', config.client_secret)
	}

	return params.toString()
}

/**
 * Builds the URL-encoded payload for refresh token grant
 */
export function buildTokenRefreshBody(config: IKeycloakConfig, refreshToken: string): string {
	const params = new URLSearchParams({
		grant_type: 'refresh_token',
		client_id: config.client_id,
		refresh_token: String(refreshToken)
	})

	if (config.client_secret) {
		params.set('client_secret', config.client_secret)
	}

	return params.toString()
}

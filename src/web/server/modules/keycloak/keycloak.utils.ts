// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { IKeycloakConfig } from '@@/shared/types/cluster.type'

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

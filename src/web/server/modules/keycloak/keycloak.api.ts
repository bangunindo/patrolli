// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { IKeycloakConfig } from '@@/shared/types/cluster.type'
import { executeHttpRequest } from '@@/server/services/apibase.service'
import { buildTokenExchangeBody, buildTokenRefreshBody } from './keycloak.utils'
import type { IKeycloakTokenResponse } from './keycloak.type'

/**
 * Exchanges authorization code for Keycloak tokens via token endpoint
 */
export async function exchangeKeycloakCode(
	config: IKeycloakConfig,
	code: string,
	redirectUri?: string
) {
	const tokenEndpoint = `${config.host}/protocol/openid-connect/token`
	const body = buildTokenExchangeBody(config, code, redirectUri)

	return executeHttpRequest<IKeycloakTokenResponse>({
		method: 'POST',
		url: tokenEndpoint,
		headers: {
			'Content-Type': 'application/x-www-form-urlencoded'
		},
		data: body
	})
}

/**
 * Refreshes an expired Keycloak token using refresh_token
 */
export async function refreshKeycloakToken(config: IKeycloakConfig, refreshToken: string) {
	const tokenEndpoint = `${config.host}/protocol/openid-connect/token`
	const body = buildTokenRefreshBody(config, refreshToken)

	return executeHttpRequest<IKeycloakTokenResponse>({
		method: 'POST',
		url: tokenEndpoint,
		headers: {
			'Content-Type': 'application/x-www-form-urlencoded'
		},
		data: body
	})
}

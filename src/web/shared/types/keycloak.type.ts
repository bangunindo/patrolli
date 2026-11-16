// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

export interface IKeycloakTokenResponse {
	access_token: string
	expires_in?: number
	refresh_expires_in?: number
	refresh_token?: string
	token_type?: string
	id_token?: string
	session_state?: string
	scope?: string
	error?: string
	error_description?: string
}

export interface IKeycloakExchangeCodeParams {
	code: string
	redirectUri?: string
}

export interface IKeycloakRefreshTokenParams {
	refreshToken: string
}

export interface IKeycloakLoginResponse {
	url: string
	host: string
	client_id: string
	redirect_uri: string
}

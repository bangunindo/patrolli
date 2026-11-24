// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

export interface IOidcPublicConfig {
	enabled: boolean
	display_name: string
}

export interface IOidcLoginResponse {
	url: string
	host?: string
	client_id?: string
	redirect_uri?: string
}

export interface IOidcTokenSet {
	access_token?: string
	id_token?: string
	refresh_token?: string
	expires_in?: number
	token_type?: string
	scope?: string
}

export interface IOidcTransaction {
	codeVerifier: string
	state: string
	nonce: string
	redirectUri: string
	expiresAt?: number
}

// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { defineEventHandler, setResponseHeader, setResponseStatus } from 'h3'
import {
	clearAuthCookies,
	extractAuthUserFromTokens,
	extractRemainingTokenSeconds,
	getAuthCookies,
	isJwtExpired
} from '@@/server/utils/auth-cookies'
import { performTokenRefresh } from '@@/server/services/auth-refresh.service'

export default defineEventHandler(async (event) => {
	setResponseHeader(event, 'cache-control', 'private, no-store')
	const cookies = getAuthCookies(event)
	let accessToken = cookies.access_token
	let idToken = cookies.id_token

	if ((!accessToken || isJwtExpired(accessToken)) && cookies.refresh_token) {
		const refreshResult = await performTokenRefresh(event, {
			refreshToken: cookies.refresh_token,
			authProvider: cookies.auth_provider || 'local',
			clusterHeader: cookies.active_cluster_url
		})

		if (refreshResult.ok && refreshResult.tokens?.access_token) {
			accessToken = refreshResult.tokens.access_token
			idToken =
				refreshResult.tokens.id_token ??
				event.context?._refreshedIdToken ??
				(idToken && !isJwtExpired(idToken) ? idToken : undefined)
		} else if (!refreshResult.isSessionInvalid) {
			setResponseStatus(event, refreshResult.status)
			return refreshResult.errorPayload
		}
	}

	if (!accessToken || isJwtExpired(accessToken)) {
		clearAuthCookies(event)
		setResponseStatus(event, 401)
		return {
			status_code: 401,
			success: false,
			is_session_invalid: true,
			error: {
				title: 'Unauthorized',
				detail: 'No active session found'
			}
		}
	}

	const user = extractAuthUserFromTokens(accessToken, idToken)
	const expiresIn = extractRemainingTokenSeconds(accessToken)
	return {
		status_code: 200,
		success: true,
		data: {
			user: user || {},
			auth_provider: cookies.auth_provider || 'local',
			...(expiresIn !== undefined ? { expires_in: expiresIn } : {})
		}
	}
})

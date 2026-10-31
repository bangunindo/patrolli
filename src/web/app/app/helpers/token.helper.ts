// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { jwtDecode } from 'jwt-decode'
import cookieHelper from '@/app/helpers/cookie.helper'

interface DecodedToken {
	exp: number
	preferred_username?: string
}

const tokenHelper = {
	getAccessToken(): string | null {
		return cookieHelper.get('access_token')
	},

	getRefreshToken(): string | null {
		return cookieHelper.get('refresh_token')
	},

	getIdToken(): string | null {
		return cookieHelper.get('id_token')
	},

	setTokens(tokens: { access_token?: string; refresh_token?: string; id_token?: string }) {
		if (tokens.access_token) {
			cookieHelper.set('access_token', tokens.access_token, 1)
		}
		if (tokens.refresh_token) {
			cookieHelper.set('refresh_token', tokens.refresh_token, 7)
		}
		if (tokens.id_token) {
			cookieHelper.set('id_token', tokens.id_token, 1)
		}
	},

	clearTokens() {
		cookieHelper.remove('access_token')
		cookieHelper.remove('refresh_token')
		cookieHelper.remove('id_token')
	},

	decodeToken(customToken?: string): DecodedToken {
		const access_token: string | null = customToken || this.getAccessToken()
		if (!access_token) return { exp: 0 }
		try {
			return jwtDecode(access_token)
		} catch {
			return { exp: 0 }
		}
	},

	getExpirationDateFromToken(): number {
		if (this.decodeToken().exp === 0) return 0
		try {
			const decodedToken: DecodedToken = this.decodeToken()
			return decodedToken.exp
		} catch {
			return 0
		}
	},

	isTokenExpired(): boolean {
		if (this.getExpirationDateFromToken() === 0) return false
		try {
			const expDate: number = this.getExpirationDateFromToken()
			const currentTime: number = Date.now() / 1000
			return currentTime >= expDate
		} catch {
			return false
		}
	}
}
export default tokenHelper

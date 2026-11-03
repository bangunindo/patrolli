// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type {
	IAuthResponse,
	ILoginBody,
	ILoginResponse,
	ILogoutResponse
} from '@@/shared/types/auth.type'
import cookieHelper from '@/app/helpers/cookie.helper'
import { httpService } from '@/app/services/http.service'

const authApi = {
	path: '/auth',

	async login(payload?: ILoginBody, clusterURLs?: string[]): Promise<ILoginResponse | undefined> {
		const path = `${this.path}/login`
		return httpService.post(path, payload, undefined, clusterURLs)
	},

	async logout(): Promise<ILogoutResponse | undefined> {
		const path = `${this.path}/logout`
		const response = await httpService.post(path)
		cookieHelper.clearAll()
		return response
	},

	async changePassword(payload?: ILoginBody): Promise<ILogoutResponse | undefined> {
		const path = `${this.path}/change-password`
		return httpService.post(path, payload)
	},

	async refreshToken(): Promise<IAuthResponse | undefined> {
		return httpService.post<IAuthResponse>('/api/auth/refresh')
	},

	async getKeycloakLoginUrl(): Promise<{ success: boolean; data?: { url: string } } | undefined> {
		return httpService.get<{ success: boolean; data?: { url: string } }>(
			'/api/auth/keycloak/login'
		)
	},

	async exchangeKeycloakCode(code: string): Promise<IAuthResponse | undefined> {
		return httpService.post<IAuthResponse>('/api/auth/keycloak/callback', { code })
	}
}

export default authApi

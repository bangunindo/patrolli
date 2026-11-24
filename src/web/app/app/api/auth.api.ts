// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type {
	AuthProvider,
	IAuthResponse,
	IAuthUser,
	ILoginBody,
	ILoginResponse,
	ILogoutResponse
} from '@@/shared/types/auth.type'
import type { IOidcLoginResponse } from '@@/shared/types/oidc.type'
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

	async refreshToken(): Promise<IAuthResponse | undefined> {
		return httpService.post<IAuthResponse>('/api/auth/refresh')
	},

	async me(): Promise<
		| {
				success: boolean
				data?: {
					user: IAuthUser
					auth_provider: AuthProvider
					expires_in?: number
				}
		  }
		| undefined
	> {
		return httpService.get('/api/auth/me', {
			suppressErrorStatuses: [401],
			rethrowErrors: true
		})
	},

	async getOidcLoginUrl(): Promise<{ success: boolean; data?: IOidcLoginResponse } | undefined> {
		return httpService.get<{ success: boolean; data?: IOidcLoginResponse }>(
			'/api/auth/oidc/login'
		)
	}
}

export default authApi

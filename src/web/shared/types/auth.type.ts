// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { LoginBody } from '@@/shared/generated/auth.type'
import type { IBaseResponse } from '@@/shared/types/api.type'
import type z from 'zod'

export type AuthProvider = 'local' | 'oidc'

export interface ILoginBody extends z.infer<typeof LoginBody> {}

export interface IAuthUser {
	preferred_username?: string
	username?: string
	sub?: string
	email?: string
	clusters?: string[]
	cluster?: string
}

export declare interface IAuthResponse extends IBaseResponse {
	data: {
		expires_in?: number
		user?: IAuthUser
	}
}

export declare interface ILoginResponse extends IAuthResponse {}

export declare interface ILogoutResponse extends IAuthResponse {}

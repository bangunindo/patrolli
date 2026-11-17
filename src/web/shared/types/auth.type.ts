// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { ChangePasswordBody, LoginBody } from '@@/shared/generated/auth.type'
import type { IBaseResponse } from '@@/shared/types/api.type'
import type { IUser } from '@@/shared/types/app.type'
import type { ICluster } from '@@/shared/types/cluster.type'
import type z from 'zod'

export type AuthProvider = 'local' | 'keycloak'

export interface IAuthCredentials {
	username?: string
	password?: string
	[key: string]: any
}

export interface IAuthRefreshPayload {
	refresh_token?: string
	auth_provider?: AuthProvider
}

export interface IAuthClusterResult {
	cluster: ICluster
	data: any
	status: number
	headers?: Headers
}

export interface ILoginBody extends z.infer<typeof LoginBody> {}
export interface IChangePasswordBody extends z.infer<typeof ChangePasswordBody> {}

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

export declare interface IProfileResponse extends IBaseResponse {
	data: IUser
}

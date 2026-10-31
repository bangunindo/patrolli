// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { ChangePasswordBody, LoginBody } from '@@/shared/generated/auth.type'
import type { IBaseResponse } from '@@/shared/types/api.type'
import type z from 'zod'

import type { IUser } from '@@/shared/types/app.type'

export interface ILoginBody extends z.infer<typeof LoginBody> {}
export interface IChangePasswordBody extends z.infer<typeof ChangePasswordBody> {}

export declare interface IAuthResponse extends IBaseResponse {
	data: {
		access_token: string
		expires_in: number
		refresh_token: string
		id_token?: string
	}
}

export declare interface ILoginResponse extends IAuthResponse {}

export declare interface ILogoutResponse extends IAuthResponse {}

export declare interface IProfileResponse extends IBaseResponse {
	data: IUser
}

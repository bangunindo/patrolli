// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import * as zod from 'zod'
import {
	CreateUserRequest,
	type ListUserResponseInner,
	type ResponseDataCreateUserResponse,
	type ResponseDataGetUserResponse,
	type ResponseDataListUserResponse,
	type ResponseDataTupleUnit,
	type ResponseDataUpdateUserResponse,
	UpdateUserRequest
} from '@@/shared/generated/patrolliAPI.schemas.type'
import { ListUsersQueryParams } from '@@/shared/generated/users.type'

export type IListUsersQueryParams = zod.input<typeof ListUsersQueryParams>
export const CreateUserFormSchema = CreateUserRequest.extend({
	password: zod.string().min(1, 'Password is required'),
	allow_sso: zod.boolean().default(true)
})

export const UpdateUserFormSchema = UpdateUserRequest.extend({
	password: zod.string().nullish().or(zod.literal(''))
})

export type CreateUserFormSchema = zod.input<typeof CreateUserFormSchema>
export type UpdateUserFormSchema = zod.input<typeof UpdateUserFormSchema>

export type {
	CreateUserRequest,
	UpdateUserRequest,
	ListUserResponseInner,
	ResponseDataCreateUserResponse,
	ResponseDataGetUserResponse,
	ResponseDataListUserResponse,
	ResponseDataTupleUnit,
	ResponseDataUpdateUserResponse
}

export type ClusterRoleName = 'Admin' | 'Editor' | 'Viewer'

export interface IClusterRoleAssignment {
	cluster: string
	role: ClusterRoleName
}

/**
 * UI User data type extending ListUserResponseInner from OpenAPI Zod schema
 * with UI-specific field augmentations such as clusterRoles and formatted last_login.
 */
export interface IUserRecord extends ListUserResponseInner {
	cluster_name?: string
	cluster_url?: string
	clusters?: string[]
	clusterRoles?: IClusterRoleAssignment[]
	last_login_formatted?: string // e.g., '2 hours ago', 'Just now', 'Never'
}

export interface IUserFilter {
	search: string
	cluster: string
	role: string
	status: string // 'all' | '1' (active) | '0' (inactive)
	authSource: string // 'all' | 'local' | 'sso'
}

export interface IUserStats {
	totalAllowSso: number
	totalDisabled: number
	totalActive?: number
	totalLogin?: number
	usersPerCluster: Record<string, number>
}

export interface IUserSummaryData {
	total_user: number
	total_active_user: number
	total_disabled_user: number
	total_allow_sso: number
	user_cluster: Record<string, number>
}

export interface IUserSummaryApiResponse {
	status_code: number
	success: boolean
	data: IUserSummaryData
}

// Aliases for backwards compatibility
export type IUserStatsData = IUserSummaryData
export type IUserStatsApiResponse = IUserSummaryApiResponse

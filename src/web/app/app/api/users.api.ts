// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type {
	CreateUserRequest,
	IListUsersQueryParams,
	IUserStatsApiResponse,
	IUserSummaryApiResponse,
	ResponseDataCreateUserResponse,
	ResponseDataGetUserResponse,
	ResponseDataListUserResponse,
	ResponseDataTupleUnit,
	ResponseDataUpdateUserResponse,
	UpdateUserRequest
} from '@@/shared/types/user.type'
import { httpService } from '@/app/services/http.service'

const usersApi = {
	path: '/api/users',

	async getUsers(
		query?: IListUsersQueryParams,
		clusterURLs?: string[]
	): Promise<ResponseDataListUserResponse | undefined> {
		return httpService.get(
			this.path,
			{
				params: query ?? {}
			},
			clusterURLs
		)
	},

	async getUser(
		userId: string,
		clusterURLs?: string[]
	): Promise<ResponseDataGetUserResponse | undefined> {
		return httpService.get(`${this.path}/${userId}`, undefined, clusterURLs)
	},

	async createUser(
		payload: CreateUserRequest,
		clusterURLs?: string[]
	): Promise<ResponseDataCreateUserResponse | undefined> {
		return httpService.post<ResponseDataCreateUserResponse>(
			this.path,
			payload,
			undefined,
			clusterURLs
		)
	},

	async updateUser(
		userId: string,
		payload: UpdateUserRequest,
		clusterURLs?: string[],
		params?: Record<string, any>
	): Promise<ResponseDataUpdateUserResponse | undefined> {
		return httpService.patch<ResponseDataUpdateUserResponse>(
			`${this.path}/${userId}`,
			payload,
			params ? { params } : undefined,
			clusterURLs
		)
	},

	async unlinkSso(
		userId: string,
		clusterURLs?: string[],
		username?: string
	): Promise<ResponseDataUpdateUserResponse | undefined> {
		return httpService.post<ResponseDataUpdateUserResponse>(
			`${this.path}/${userId}/unlink-sso`,
			username ? { username } : undefined,
			username ? { params: { username } } : undefined,
			clusterURLs
		)
	},

	async getUserSummary(clusterURLs?: string[]): Promise<IUserSummaryApiResponse | undefined> {
		return httpService.get(`${this.path}/summary`, undefined, clusterURLs)
	},

	async getUserStats(clusterURLs?: string[]): Promise<IUserStatsApiResponse | undefined> {
		return this.getUserSummary(clusterURLs)
	},

	async deleteUser(
		userId: string,
		clusterURLs?: string[],
		username?: string
	): Promise<ResponseDataTupleUnit | undefined> {
		try {
			return await httpService.delete(
				`${this.path}/${userId}`,
				username ? { params: { username } } : undefined,
				clusterURLs
			)
		} catch (error: any) {
			throw new Error(error.message || 'Failed to delete user')
		}
	}
}

export default usersApi

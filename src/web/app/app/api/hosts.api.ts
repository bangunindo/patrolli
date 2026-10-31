// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type {
	IHostForm,
	IHostListParams,
	IHostListResponse,
	IHostResponse
} from '@@/shared/types/host.type'
import { httpService } from '@/app/services/http.service'

const hostsApi = {
	path: '/api/hosts',

	async getHosts(
		query?: IHostListParams,
		clusterURLs?: string[]
	): Promise<IHostListResponse | undefined> {
		return httpService.get(
			this.path,
			{
				params: query ?? {}
			},
			clusterURLs
		)
	},

	async createHost(
		payload: IHostForm,
		clusterURLs?: string[]
	): Promise<IHostResponse | undefined> {
		return httpService.post<IHostResponse>(this.path, payload, undefined, clusterURLs)
	},

	async updateHost(
		hostId: string,
		payload: IHostForm,
		clusterURLs?: string[]
	): Promise<IHostResponse | undefined> {
		return httpService.put(`${this.path}/${hostId}`, payload, undefined, clusterURLs)
	},

	async deleteHost(hostId: string, clusterURLs?: string[]): Promise<IHostResponse | undefined> {
		try {
			return await httpService.delete(`${this.path}/${hostId}`, undefined, clusterURLs)
		} catch (error: any) {
			throw new Error(error.message || 'Failed to delete host')
		}
	}
}

export default hostsApi

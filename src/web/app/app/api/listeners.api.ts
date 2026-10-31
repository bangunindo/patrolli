// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type {
	IListenerForm,
	IListenerListParams,
	IListenerListResponse,
	IListenerResponse
} from '@@/shared/types/listener.type'
import { httpService } from '@/app/services/http.service'

const listenerApi = {
	path: '/api/listeners',

	async getListeners(
		query?: IListenerListParams,
		clusterURLs?: string[]
	): Promise<IListenerListResponse | undefined> {
		return httpService.get(
			this.path,
			{
				params: query ?? {}
			},
			clusterURLs
		)
	},

	async createListener(
		payload: IListenerForm,
		clusterURLs?: string[]
	): Promise<IListenerResponse | undefined> {
		return httpService.post(this.path, payload, undefined, clusterURLs)
	},

	async updateListener(
		listenerId: string,
		payload: IListenerForm,
		clusterURLs?: string[]
	): Promise<IListenerResponse | undefined> {
		return httpService.put(`${this.path}/${listenerId}`, payload, undefined, clusterURLs)
	},

	async deleteListener(
		listenerId: string,
		clusterURLs?: string[]
	): Promise<IListenerResponse | undefined> {
		try {
			return await httpService.delete(`${this.path}/${listenerId}`, undefined, clusterURLs)
		} catch (error: any) {
			throw new Error(error.message || 'Failed to delete listener')
		}
	}
}

export default listenerApi

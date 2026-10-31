// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type {
	IEndpointForm,
	IEndpointListParams,
	IEndpointListResponse,
	IEndpointResponse
} from '@@/shared/types/endpoint.type'
import { httpService } from '@/app/services/http.service'

const endpointsApi = {
	path: '/api/endpoints',

	async getEndpoints(
		query?: IEndpointListParams,
		clusterURLs?: string[]
	): Promise<IEndpointListResponse | undefined> {
		return httpService.get(
			this.path,
			{
				params: query ?? {}
			},
			clusterURLs
		)
	},

	async createEndpoint(
		payload: IEndpointForm,
		clusterURLs?: string[]
	): Promise<IEndpointResponse | undefined> {
		return httpService.post(this.path, payload, undefined, clusterURLs)
	},

	async updateEndpoint(
		endpointId: string,
		payload: IEndpointForm,
		clusterURLs?: string[]
	): Promise<IEndpointResponse | undefined> {
		return httpService.put(`${this.path}/${endpointId}`, payload, undefined, clusterURLs)
	},

	async deleteEndpoint(
		endpointId: string,
		clusterURLs?: string[]
	): Promise<IEndpointResponse | undefined> {
		try {
			return await httpService.delete(`${this.path}/${endpointId}`, undefined, clusterURLs)
		} catch (error: any) {
			throw new Error(error.message || 'Failed to delete endpoint')
		}
	}
}

export default endpointsApi

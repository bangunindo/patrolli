// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type {
	ISinkForm,
	ISinkListParams,
	ISinkListResponse,
	ISinkResponse
} from '@@/shared/types/sink.type'
import { httpService } from '@/app/services/http.service'

const sinksApi = {
	path: '/api/sinks',

	async getSinks(
		query?: ISinkListParams,
		clusterURLs?: string[]
	): Promise<ISinkListResponse | undefined> {
		return httpService.get(
			this.path,
			{
				params: query ?? {}
			},
			clusterURLs
		)
	},

	async createSink(
		payload: ISinkForm,
		clusterURLs?: string[]
	): Promise<ISinkResponse | undefined> {
		return httpService.post(this.path, payload, undefined, clusterURLs)
	},

	async getSinkById(sinkId: string, clusterURLs?: string[]): Promise<ISinkResponse | undefined> {
		return httpService.get(`${this.path}/${sinkId}`, undefined, clusterURLs)
	},

	async updateSink(
		sinkId: string,
		payload: ISinkForm,
		clusterURLs?: string[]
	): Promise<ISinkResponse | undefined> {
		return httpService.put(`${this.path}/${sinkId}`, payload, undefined, clusterURLs)
	},

	async deleteSink(sinkId: string, clusterURLs?: string[]): Promise<ISinkResponse | undefined> {
		try {
			return await httpService.delete(`${this.path}/${sinkId}`, undefined, clusterURLs)
		} catch (error: any) {
			throw new Error(error.message || 'Failed to delete sink')
		}
	}
}

export default sinksApi

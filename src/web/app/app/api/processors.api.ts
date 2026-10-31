// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type {
	IProcessorForm,
	IProcessorListParams,
	IProcessorListResponse,
	IProcessorResponse
} from '@@/shared/types/processor.type'
import { httpService } from '@/app/services/http.service'

const processorApi = {
	path: '/api/processors',

	async get(
		query?: IProcessorListParams,
		clusterURLs?: string[]
	): Promise<IProcessorListResponse | undefined> {
		return httpService.get(
			this.path,
			{
				params: query ?? {}
			},
			clusterURLs
		)
	},

	async create(
		payload: IProcessorForm,
		clusterURLs?: string[]
	): Promise<IProcessorResponse | undefined> {
		return httpService.post<IProcessorResponse>(this.path, payload, undefined, clusterURLs)
	},

	async update(
		id: string,
		payload: IProcessorForm,
		clusterURLs?: string[]
	): Promise<IProcessorResponse | undefined> {
		return httpService.put(`${this.path}/${id}`, payload, undefined, clusterURLs)
	},

	async delete(id: string, clusterURLs?: string[]): Promise<IProcessorResponse | undefined> {
		try {
			return await httpService.delete(`${this.path}/${id}`, undefined, clusterURLs)
		} catch (error: any) {
			throw new Error(error.message || 'Failed to delete processor')
		}
	}
}

export default processorApi

// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type {
	IItemForm,
	IItemListParams,
	IItemListResponse,
	IItemResponse
} from '@@/shared/types/item.type'
import { httpService } from '@/app/services/http.service'

const itemsApi = {
	path: '/api/items',

	async getItems(
		query?: IItemListParams,
		clusterURLs?: string[]
	): Promise<IItemListResponse | undefined> {
		return httpService.get(
			this.path,
			{
				params: query ?? {}
			},
			clusterURLs
		)
	},

	async createItemForHost(
		hostId: string,
		payload?: IItemForm,
		clusterURLs?: string[]
	): Promise<IItemResponse | undefined> {
		return httpService.post(
			`${this.path}/host/${hostId}`,
			payload ?? {},
			undefined,
			clusterURLs
		)
	},

	async createItemForTemplate(
		templateId: string,
		payload?: IItemForm,
		clusterURLs?: string[]
	): Promise<IItemResponse | undefined> {
		return httpService.post(
			`${this.path}/template/${templateId}`,
			payload ?? {},
			undefined,
			clusterURLs
		)
	},

	async getItemById(itemId: string, clusterURLs?: string[]): Promise<IItemResponse | undefined> {
		return httpService.get(`${this.path}/${itemId}`, {}, clusterURLs)
	},

	async updateItemHost(
		itemId: string,
		payload: IItemForm,
		clusterURLs?: string[]
	): Promise<IItemResponse | undefined> {
		return httpService.put(`${this.path}/${itemId}/host`, payload, undefined, clusterURLs)
	},

	async updateItemApliedTemplate(
		itemId: string,
		payload: IItemForm,
		clusterURLs?: string[]
	): Promise<IItemResponse | undefined> {
		return httpService.put(
			`${this.path}/${itemId}/applied-template`,
			payload,
			undefined,
			clusterURLs
		)
	},

	async updateItemTemplate(
		itemId: string,
		payload: IItemForm,
		clusterURLs?: string[]
	): Promise<IItemResponse | undefined> {
		return httpService.put(`${this.path}/${itemId}/template`, payload, undefined, clusterURLs)
	},

	async deleteItem(itemId: string, clusterURLs?: string[]): Promise<IItemResponse | undefined> {
		try {
			return await httpService.delete(`${this.path}/${itemId}`, undefined, clusterURLs)
		} catch (error: any) {
			throw new Error(error.message || 'Failed to delete item')
		}
	}
}

export default itemsApi

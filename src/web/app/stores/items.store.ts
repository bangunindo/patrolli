// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { reactive, ref } from 'vue'

import { defineStore } from 'pinia'

import itemsApi from '@/app/api/items.api'

import endpointsApi from '@/app/api/endpoints.api'
import hostsApi from '@/app/api/hosts.api'
import listenerApi from '@/app/api/listeners.api'
import templatesApi from '@/app/api/templates.api'
import { FormMode } from '@@/shared/types/app.type'
import type { IEndpointSelect } from '@@/shared/types/endpoint.type'
import type { IHostSelect } from '@@/shared/types/host.type'
import {
	ItemType,
	type IItem,
	type IItemForm,
	type IItemListParams,
	type IItems
} from '@@/shared/types/item.type'
import type { IListenerSelect } from '@@/shared/types/listener.type'
import type { ITemplateSelect } from '@@/shared/types/template.type'

export const useItemsStore = defineStore(
	'items',
	() => {
		const pagination = reactive({
			rows: 10,
			currentPage: 1,
			totalRecords: 0
		})
		const filter = reactive<IItemListParams>({
			name: undefined,
			limit: pagination.rows,
			offset: (pagination.currentPage - 1) * pagination.rows,
			order_by: undefined,
			order_dir: undefined,
			types: undefined,
			listener_ids: undefined,
			endpoint_ids: undefined,
			exclusivity: ['host', 'template_on_host'],
			collect_item_stats: true
		})

		const items = ref<IItems[]>([])
		const currentItem = ref<IItem | null>(null)
		const loading = reactive({
			table: false,
			form: false,
			endpoints: false,
			listeners: false
		})
		const error = ref<string | null>(null)
		const selection = reactive<{
			hosts?: IHostSelect[]
			templates?: ITemplateSelect[]
			types?: any[]
			endpoints?: IEndpointSelect[]
			listeners?: IListenerSelect[]
		}>({
			hosts: undefined,
			templates: undefined,
			types: Object.entries(ItemType).map(([key, value]) => ({
				label: key,
				value: value
			})),
			endpoints: undefined,
			listeners: undefined
		})

		const resetFilter = () => {
			filter.name = undefined
			filter.order_by = undefined
			filter.order_dir = undefined
			filter.types = undefined
			filter.listener_ids = undefined
			filter.endpoint_ids = undefined
			filter.offset = 0
			pagination.currentPage = 1
		}

		const fetchItems = async (clusterURLs?: string[]) => {
			loading.table = true
			error.value = null

			try {
				const response = await itemsApi.getItems(filter, clusterURLs)
				if (response?.success) {
					items.value = response.data.items
					pagination.totalRecords = response.data.total ?? 0
				}
			} catch (err: any) {
				error.value = err.message || 'Failed to fetch items'
			} finally {
				loading.table = false
			}
		}

		const fetchHosts = async (clusterURLs?: string[]) => {
			loading.form = true
			error.value = null

			try {
				const response = await hostsApi.getHosts({ limit: 100 }, clusterURLs)
				if (response?.success) {
					selection.hosts = response.data.hosts.map((host) => ({
						...host,
						value: host.host_id,
						label: host.name
					}))
				}
			} catch (err: any) {
				error.value = err.message || 'Failed to fetch templates'
			} finally {
				loading.form = false
			}
		}

		const fetchTemplates = async (clusterURLs?: string[]) => {
			loading.form = true
			error.value = null

			try {
				const response = await templatesApi.getTemplates({ limit: 100 }, clusterURLs)
				if (response?.success) {
					selection.templates = response.data.templates.map((template) => ({
						...template,
						value: template.template_id,
						label: template.name
					}))
				}
			} catch (err: any) {
				error.value = err.message || 'Failed to fetch templates'
			} finally {
				loading.form = false
			}
		}

		const selectionRequestIds = { endpoints: 0, listeners: 0 }

		const fetchSelection = async <T>(
			key: 'endpoints' | 'listeners',
			search: string | undefined,
			fetcher: (query: { limit: number; name?: string }) => Promise<T[] | null>,
			onSuccess: (data: T[]) => void,
			fallbackError: string
		) => {
			const requestId = ++selectionRequestIds[key]
			loading[key] = true
			error.value = null

			try {
				const trimmed = search?.trim()
				const query = { limit: 100, ...(trimmed ? { name: trimmed } : {}) }
				const data = await fetcher(query)
				if (requestId !== selectionRequestIds[key]) return
				if (data) onSuccess(data)
			} catch (err: any) {
				if (requestId !== selectionRequestIds[key]) return
				error.value = err.message || fallbackError
			} finally {
				if (requestId === selectionRequestIds[key]) {
					loading[key] = false
				}
			}
		}

		const fetchEndpoints = (clusterURLs?: string[], search?: string) =>
			fetchSelection(
				'endpoints',
				search,
				async (query) => {
					const response = await endpointsApi.getEndpoints(query, clusterURLs)
					return response?.success ? response.data.endpoints : null
				},
				(endpoints) => {
					selection.endpoints = endpoints.map((endpoint) => ({
						...endpoint,
						value: endpoint.endpoint_id,
						label: endpoint.name
					}))
				},
				'Failed to fetch endpoints'
			)

		const fetchListeners = (clusterURLs?: string[], search?: string) =>
			fetchSelection(
				'listeners',
				search,
				async (query) => {
					const response = await listenerApi.getListeners(query, clusterURLs)
					return response?.success ? response.data.listeners : null
				},
				(listeners) => {
					selection.listeners = listeners.map((listener) => ({
						...listener,
						value: listener.listener_id,
						label: listener.name
					}))
				},
				'Failed to fetch listeners'
			)

		const createItemForHost = async (
			hostId: string,
			payload?: IItemForm,
			clusterURLs?: string[]
		) => {
			loading.form = true
			error.value = null
			try {
				const response = await itemsApi.createItemForHost(hostId, payload, clusterURLs)
				if (response && response.success) {
					await fetchItems()
					return response
				}
			} catch (err: any) {
				error.value = err.message || 'Failed to create item'
				throw err
			} finally {
				loading.form = false
			}
		}

		const createItemForTemplate = async (
			templateId: string,
			payload?: IItemForm,
			clusterURLs?: string[]
		) => {
			loading.form = true
			error.value = null
			try {
				const response = await itemsApi.createItemForTemplate(
					templateId,
					payload,
					clusterURLs
				)
				if (response && response.success) {
					await fetchItems()
					return response
				}
			} catch (err: any) {
				error.value = err.message || 'Failed to create item'
				throw err
			} finally {
				loading.form = false
			}
		}

		const fetchItemById = async (itemId: string, clusterURLs?: string[]) => {
			loading.form = true
			error.value = null
			try {
				const response = await itemsApi.getItemById(itemId, clusterURLs)
				if (response?.success) {
					currentItem.value = response.data ?? null
					return response.data
				}
			} catch (err: any) {
				error.value = err.message || 'Failed to fetch item'
			} finally {
				loading.form = false
			}
		}

		const updateItemHost = async (
			itemId: string,
			payload: IItemForm,
			clusterURLs?: string[]
		) => {
			loading.form = true
			error.value = null
			try {
				const response = await itemsApi.updateItemHost(itemId, payload, clusterURLs)

				if (response && response.success) {
					await fetchItems()
					return response
				}
			} catch (err: any) {
				error.value = err.message || 'Failed to update item'
				throw err
			} finally {
				loading.form = false
			}
		}

		const updateItemApliedTemplate = async (
			itemId: string,
			payload: IItemForm,
			clusterURLs?: string[]
		) => {
			loading.form = true
			error.value = null
			try {
				const response = await itemsApi.updateItemApliedTemplate(
					itemId,
					payload,
					clusterURLs
				)

				if (response && response.success) {
					await fetchItems()
					return response
				}
			} catch (err: any) {
				error.value = err.message || 'Failed to update item'
				throw err
			} finally {
				loading.form = false
			}
		}

		const updateItemTemplate = async (itemId: string, payload: any, clusterURLs?: string[]) => {
			loading.form = true
			error.value = null
			try {
				const response = await itemsApi.updateItemTemplate(itemId, payload, clusterURLs)

				if (response && response.success) {
					await fetchItems()
					return response
				}
			} catch (err: any) {
				error.value = err.message || 'Failed to update item'
				throw err
			} finally {
				loading.form = false
			}
		}

		const deleteItem = async (itemId: string, clusterURLs?: string[]) => {
			loading.form = true
			error.value = null
			try {
				const response = await itemsApi.deleteItem(itemId, clusterURLs)
				if (response && response.success) {
					await fetchItems()
					return response
				}
			} catch (err: any) {
				error.value = err.message || 'Failed to delete item'
				throw err
			} finally {
				loading.form = false
			}
		}

		const itemsModalShow = ref<boolean>(false)
		const itemsFormMode = ref<FormMode>(FormMode.ADD)
		const toggleItemsModalForm = (
			value: boolean,
			formMode: FormMode = FormMode.ADD,
			item: IItem | null = null
		) => {
			itemsModalShow.value = value
			itemsFormMode.value = formMode
			currentItem.value = item
		}

		const itemsTemplateModalShow = ref<boolean>(false)
		const toggleItemsTemplateModalForm = (
			value: boolean,
			formMode: FormMode = FormMode.ADD,
			item: IItem | null = null
		) => {
			itemsFormMode.value = formMode
			currentItem.value = item
			itemsTemplateModalShow.value = value
		}

		return {
			pagination,
			filter,
			items,
			currentItem,
			loading,
			error,
			selection,
			resetFilter,
			fetchItems,
			fetchHosts,
			fetchTemplates,
			fetchEndpoints,
			fetchListeners,
			createItemForHost,
			createItemForTemplate,
			fetchItemById,
			updateItemHost,
			updateItemApliedTemplate,
			updateItemTemplate,
			deleteItem,
			itemsModalShow,
			itemsFormMode,
			toggleItemsModalForm,
			itemsTemplateModalShow,
			toggleItemsTemplateModalForm
		}
	},
	{ persist: false }
)

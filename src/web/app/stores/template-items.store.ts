// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { reactive, ref } from 'vue'

import { defineStore } from 'pinia'

import itemsApi from '@/app/api/items.api'
import templatesApi from '@/app/api/templates.api'

import { FormMode } from '@@/shared/types/app.type'
import { type IItem, type IItemListParams, type IItems } from '@@/shared/types/item.type'
import type { ITemplate } from '@@/shared/types/template.type'

export const useTemplateItemsStore = defineStore(
	'template-items',
	() => {
		const templateId = ref<string>('')
		const currentTemplate = ref<ITemplate | null>(null)

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
			template_ids: undefined,
			exclusivity: ['template']
		})

		const items = ref<IItems[]>([])
		const currentItem = ref<IItem | null>(null)
		const loading = reactive({
			table: false,
			form: false,
			template: false
		})
		const error = ref<string | null>(null)

		const setTemplateId = (id: string) => {
			templateId.value = id
			filter.template_ids = [id]
		}

		const fetchTemplate = async (clusterURLs?: string[]) => {
			loading.template = true
			error.value = null
			try {
				const response = await templatesApi.getTemplateById(templateId.value, clusterURLs)
				if (response?.success) {
					currentTemplate.value = response.data ?? null
					return response.data
				}
			} catch (err: any) {
				error.value = err.message || 'Failed to fetch template'
			} finally {
				loading.template = false
			}
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

		const deleteItem = async (itemId: string, clusterURLs?: string[]) => {
			loading.form = true
			error.value = null
			try {
				const response = await itemsApi.deleteItem(itemId, clusterURLs)
				if (response && response.success) {
					items.value = items.value.filter((i) => i.item_id !== itemId)
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

		const resetStore = () => {
			templateId.value = ''
			currentTemplate.value = null
			items.value = []
			currentItem.value = null
			pagination.currentPage = 1
			pagination.totalRecords = 0
			filter.name = undefined
			filter.template_ids = undefined
			filter.offset = 0
			error.value = null
		}

		return {
			templateId,
			currentTemplate,
			pagination,
			filter,
			items,
			currentItem,
			loading,
			error,
			setTemplateId,
			fetchTemplate,
			fetchItems,
			fetchItemById,
			deleteItem,
			itemsModalShow,
			itemsFormMode,
			toggleItemsModalForm,
			resetStore
		}
	},
	{ persist: false }
)

// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { reactive, ref } from 'vue'

import { defineStore } from 'pinia'

import processorApi from '@/app/api/processors.api'

import { FormMode, type ISelectOption } from '@@/shared/types/app.type'
import type { IItemSelect } from '@@/shared/types/item.type'
import {
	ProcessorAttributeFilter,
	ProcessorType,
	ProcessorTypeLabels,
	type IProcessor,
	type IProcessorForm,
	type IProcessorListParams,
	type IProcessorSelect
} from '@@/shared/types/processor.type'

export const useProcessorStore = defineStore(
	'processor',
	() => {
		const pagination = reactive({
			rows: 10,
			currentPage: 1,
			totalRecords: 0
		})
		const filter = reactive<IProcessorListParams>({
			name: undefined,
			limit: pagination.rows,
			offset: (pagination.currentPage - 1) * pagination.rows,
			order_by: undefined,
			order_dir: undefined
		})

		const processors = ref<IProcessor[]>([])
		const currentProcessor = ref<IProcessor | null>(null)
		const loading = reactive({
			table: false,
			form: false
		})
		const error = ref<string | null>(null)
		const selection = reactive<{
			modules: ISelectOption[]
			items?: IItemSelect[]
			processors?: IProcessorSelect[]
			attributeFilter: ISelectOption[]
		}>({
			modules: Object.entries(ProcessorType).map(([key, value]) => ({
				label: ProcessorTypeLabels[value] || key,
				value: value
			})),
			items: undefined,
			processors: undefined,
			attributeFilter: ProcessorAttributeFilter.map((value: string) => ({
				label: value.toUpperCase(),
				value: value
			}))
		})

		const fetchProcessors = async (clusterURLs?: string[]) => {
			loading.table = true
			error.value = null
			try {
				const response = await processorApi.get(filter, clusterURLs)
				if (response?.success) {
					processors.value = response.data.processors
					pagination.totalRecords = response.data.total ?? 0
				}
			} catch (err: any) {
				error.value = err.message || 'Failed to fetch processors'
			} finally {
				loading.table = false
			}
		}

		const fetchProcessorById = async (processorId: string, clusterURLs?: string[]) => {
			loading.form = true
			error.value = null
			try {
				const response = await processorApi.get({ name: undefined }, clusterURLs)
				if (response?.success) {
					const data =
						response.data.processors.find((p) => p.processor_id === processorId) ?? null
					currentProcessor.value = data
					return data
				}
			} catch (err: any) {
				error.value = err.message || 'Failed to fetch processor'
			} finally {
				loading.form = false
			}
		}

		const createProcessor = async (payload: IProcessorForm, clusterURLs?: string[]) => {
			loading.form = true
			error.value = null
			try {
				const response = await processorApi.create(payload, clusterURLs)
				if (response && response.success) {
					await fetchProcessors()
					return response
				}
			} catch (err: any) {
				error.value = err.message || 'Failed to create processor'
				throw err
			} finally {
				loading.form = false
			}
		}

		const updateProcessor = async (
			id: string,
			payload: IProcessorForm,
			clusterURLs?: string[]
		) => {
			loading.form = true
			error.value = null
			try {
				const response = await processorApi.update(id, payload, clusterURLs)
				if (response && response.success) {
					await fetchProcessors() // Refresh the list
					return response
				}
			} catch (err: any) {
				error.value = err.message || 'Failed to update processor'
				throw err
			} finally {
				loading.form = false
			}
		}

		const deleteProcessor = async (processorId: string, clusterURLs?: string[]) => {
			loading.form = true
			error.value = null
			try {
				const response = await processorApi.delete(processorId, clusterURLs)
				if (response && response.success) {
					return response
				}
			} catch (err: any) {
				error.value = err.message || 'Failed to delete processor'
				throw err
			} finally {
				loading.form = false
			}
		}

		const processorModalShow = ref<boolean>(false)
		const processorFormMode = ref<FormMode>(FormMode.ADD)
		const customSubmitHandler = ref<((payload: any) => Promise<any> | any) | null>(null)
		const lockedCluster = ref<string | null>(null)
		const hideInputItemsAndProcessors = ref<boolean>(false)
		const toggleProcessorModalForm = (
			value: boolean,
			formMode: FormMode = FormMode.ADD,
			processor: IProcessor | null = null,
			customHandler: ((payload: any) => Promise<any> | any) | null = null,
			options?: {
				lockedCluster?: string | null
				hideInputItemsAndProcessors?: boolean
			}
		) => {
			processorModalShow.value = value
			processorFormMode.value = formMode
			currentProcessor.value = processor
			customSubmitHandler.value = value ? customHandler : null
			lockedCluster.value = value ? options?.lockedCluster || null : null
			hideInputItemsAndProcessors.value = value
				? !!options?.hideInputItemsAndProcessors
				: false
			loading.form = false
		}

		const setDetailCanvas = async (payload) => {
			try {
				const response = await processorApi.get({ name: payload.name })

				if (response?.success) {
					const { processors } = response.data
					const data =
						processors.find(
							(h) => h.processor_id == (payload.processor_id || payload.pipeline_id)
						) ?? null

					processorFormMode.value = FormMode.UPDATE
					currentProcessor.value = data
				}
			} catch (err: any) {
				error.value = err.message || 'Failed to fetch processors'
			} finally {
				loading.table = false
			}
		}

		return {
			filter,
			pagination,
			processors,
			currentProcessor,
			loading,
			error,
			fetchProcessors,
			fetchProcessorById,
			createProcessor,
			updateProcessor,
			deleteProcessor,
			processorModalShow,
			processorFormMode,
			customSubmitHandler,
			lockedCluster,
			hideInputItemsAndProcessors,
			toggleProcessorModalForm,
			selection,
			setDetailCanvas
		}
	},
	{ persist: false }
)

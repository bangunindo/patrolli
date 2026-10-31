// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { reactive, ref } from 'vue'

import { defineStore } from 'pinia'

import sinksApi from '@/app/api/sinks.api'

import { FormMode } from '@@/shared/types/app.type'
import type { IItemSelect } from '@@/shared/types/item.type'
import type { IProcessorSelect } from '@@/shared/types/processor.type'
import {
	SinkChanelCompression,
	SinkChanelType,
	SinkConfigType,
	type ISink,
	type ISinkForm,
	type ISinkListParams,
	type ISinks
} from '@@/shared/types/sink.type'

export const useSinksStore = defineStore(
	'sinks',
	() => {
		const pagination = reactive({
			rows: 10,
			currentPage: 1,
			totalRecords: 0
		})
		const filter = reactive<ISinkListParams>({
			name: undefined,
			limit: pagination.rows,
			offset: (pagination.currentPage - 1) * pagination.rows,
			order_by: undefined,
			order_dir: undefined
		})
		const sinks = ref<ISinks[]>([])
		const currentSink = ref<ISink | null>(null)
		const loading = reactive({
			table: false,
			form: false
		})
		const error = ref<string | null>(null)
		const selection = reactive<{
			types?: any[]
			compressions?: any[]
			channelTypes?: any[]
			items?: IItemSelect[]
			processors?: IProcessorSelect[]
		}>({
			types: Object.entries(SinkConfigType).map(([key, value]) => ({
				label: key.replaceAll('_', ' '),
				value: value
			})),
			compressions: Object.entries(SinkChanelCompression).map(([key, value]) => ({
				label: key,
				value: value
			})),
			channelTypes: Object.entries(SinkChanelType).map(([key, value]) => ({
				label: key,
				value: value
			})),
			items: undefined,
			processors: undefined
		})

		const fetchSinks = async (clusterURLs?: string[]) => {
			loading.table = true
			error.value = null

			try {
				const response = await sinksApi.getSinks(filter, clusterURLs)
				if (response?.success) {
					sinks.value = response.data.sinks
					pagination.totalRecords = response.data.total
				}
			} catch (err: any) {
				error.value = err.message || 'Failed to fetch sinks'
			} finally {
				loading.table = false
			}
		}

		const fetchSinkById = async (id: string, clusterURLs?: string[]) => {
			loading.table = true
			error.value = null

			try {
				const response = await sinksApi.getSinkById(id, clusterURLs)
				if (response?.success) {
					currentSink.value = response.data ?? null
				}
			} catch (err: any) {
				error.value = err.message || 'Failed to fetch sinks'
			} finally {
				loading.table = false
			}
		}

		const createSink = async (payload: ISinkForm, clusterURLs?: string[]) => {
			loading.form = true
			error.value = null
			try {
				const response = await sinksApi.createSink(payload, clusterURLs)
				if (response && response.success) {
					await fetchSinks()
					return response
				}
			} catch (err: any) {
				error.value = err.message || 'Failed to create sink'
				throw err
			} finally {
				loading.form = false
			}
		}

		const updateSink = async (sinkId: string, payload: ISinkForm, clusterURLs?: string[]) => {
			loading.form = true
			error.value = null
			try {
				const response = await sinksApi.updateSink(sinkId, payload, clusterURLs)
				if (response && response.success) {
					await fetchSinks()
					return response
				}
			} catch (err: any) {
				error.value = err.message || 'Failed to update sink'
				throw err
			} finally {
				loading.form = false
			}
		}

		const deleteSink = async (sinkId: string, clusterURLs?: string[]) => {
			loading.form = true
			error.value = null
			try {
				const response = await sinksApi.deleteSink(sinkId, clusterURLs)
				if (response && response.success) {
					sinks.value = sinks.value.filter((h) => h.sink_id !== sinkId)
					return response
				}
			} catch (err: any) {
				error.value = err.message || 'Failed to delete sink'
				throw err
			} finally {
				loading.form = false
			}
		}

		const sinkModalShow = ref<boolean>(false)
		const sinkFormMode = ref<FormMode>(FormMode.ADD)
		const toggleSinkModalForm = (
			value: boolean,
			formMode: FormMode = FormMode.ADD,
			sink: ISink | null = null
		) => {
			sinkModalShow.value = value
			sinkFormMode.value = formMode
			currentSink.value = sink
		}

		const setDetailCanvas = async (payload) => {
			sinkFormMode.value = FormMode.UPDATE

			try {
				const response = await sinksApi.getSinkById(payload.sink_id)

				if (response?.success) {
					currentSink.value = response.data ?? null
				}
			} catch (err: any) {
				error.value = err.message || 'Failed to fetch hosts'
			} finally {
				loading.table = false
			}
		}

		return {
			pagination,
			filter,
			sinks,
			currentSink,
			loading,
			error,

			selection,
			fetchSinks,
			fetchSinkById,
			createSink,
			updateSink,
			deleteSink,

			sinkModalShow,
			sinkFormMode,
			toggleSinkModalForm,
			setDetailCanvas
		}
	},
	{ persist: false }
)

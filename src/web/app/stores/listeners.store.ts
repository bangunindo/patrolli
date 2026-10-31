// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { reactive, ref } from 'vue'

import { defineStore } from 'pinia'

import listenersApi from '@/app/api/listeners.api'

import hostsApi from '@/app/api/hosts.api'
import { FormMode } from '@@/shared/types/app.type'
import type { IHostSelect } from '@@/shared/types/host.type'
import {
	ListenerType,
	type IListener,
	type IListenerForm,
	type IListenerListParams
} from '@@/shared/types/listener.type'

export const useListenerStore = defineStore(
	'listeners',
	() => {
		const pagination = reactive({
			rows: 10,
			currentPage: 1,
			totalRecords: 0
		})
		const filter = reactive<IListenerListParams>({
			name: undefined,
			limit: pagination.rows,
			offset: (pagination.currentPage - 1) * pagination.rows,
			order_by: undefined,
			order_dir: undefined
		})
		const listeners = ref<IListener[]>([])
		const currentListener = ref<IListener | null>()
		const loading = reactive({
			table: false,
			form: false
		})
		const error = ref<any>({})
		const selection = reactive<{ hosts: IHostSelect[]; types: any[] }>({
			hosts: [],
			types: Object.entries(ListenerType).map(([key, value]) => ({
				label: key,
				value: value
			}))
		})

		const fetchListeners = async () => {
			loading.table = true
			error.value = null

			try {
				const response = await listenersApi.getListeners(filter)
				if (response?.success) {
					listeners.value = response.data.listeners
					pagination.totalRecords = response.data.total ?? 0
				}
			} catch (err: any) {
				error.value = err.message || 'Failed to fetch listeners'
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

		const createListener = async (payload: IListenerForm, clusterURLs?: string[]) => {
			loading.form = true
			error.value = null
			try {
				const response = await listenersApi.createListener(payload, clusterURLs)
				if (response && response.success) {
					await fetchListeners()
					return response
				}
			} catch (err: any) {
				error.value = err.message || 'Failed to create listener'
				throw err
			} finally {
				loading.form = false
			}
		}

		const updateListener = async (
			listenerId: string,
			payload: IListenerForm,
			clusterURLs?: string[]
		) => {
			loading.form = true
			error.value = null
			try {
				const response = await listenersApi.updateListener(listenerId, payload, clusterURLs)
				if (response && response.success) {
					await fetchListeners()
					return response
				}
			} catch (err: any) {
				error.value = err.message || 'Failed to update listener'
				throw err
			} finally {
				loading.form = false
			}
		}

		const deleteListener = async (listenerId: string, clusterURLs?: string[]) => {
			loading.form = true
			error.value = null
			try {
				const response = await listenersApi.deleteListener(listenerId, clusterURLs)
				if (response && response.success) {
					listeners.value = listeners.value.filter((l) => l.listener_id !== listenerId)
					return response
				}
			} catch (err: any) {
				error.value = err.message || 'Failed to delete listener'
				throw err
			} finally {
				loading.form = false
			}
		}

		const listenerModalShow = ref<boolean>(false)
		const listenerFormMode = ref<FormMode>(FormMode.ADD)
		const toggleListenerModalForm = (
			value: boolean,
			formMode: FormMode = FormMode.ADD,
			listener: IListener | null = null
		) => {
			listenerModalShow.value = value
			listenerFormMode.value = formMode
			currentListener.value = listener
		}

		const setDetailCanvas = async (payload) => {
			listenerFormMode.value = FormMode.UPDATE

			try {
				const response = await listenersApi.getListeners({ name: payload.name })

				if (response?.success) {
					const { listeners } = response.data
					const data = listeners.find((h) => h.listener_id == payload.listener_id) ?? null
					currentListener.value = data
				}
			} catch (err: any) {
				error.value = err.message || 'Failed to fetch hosts'
			} finally {
				loading.table = false
			}
		}

		return {
			filter,
			pagination,
			listeners,
			currentListener,
			loading,
			error,
			selection,
			fetchListeners,
			fetchHosts,
			createListener,
			updateListener,
			deleteListener,
			listenerModalShow,
			listenerFormMode,
			toggleListenerModalForm,
			setDetailCanvas
		}
	},
	{ persist: false }
)

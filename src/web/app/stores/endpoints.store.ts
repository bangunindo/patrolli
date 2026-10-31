// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { reactive, ref } from 'vue'

import { defineStore } from 'pinia'

import endpointsApi from '@/app/api/endpoints.api'

import hostsApi from '@/app/api/hosts.api'
import { FormMode } from '@@/shared/types/app.type'
import {
	EndpointType,
	type IEndpoint,
	type IEndpointForm,
	type IEndpointListParams
} from '@@/shared/types/endpoint.type'
import type { IHostSelect } from '@@/shared/types/host.type'

export const useEndpointsStore = defineStore(
	'endpoints',
	() => {
		const pagination = reactive({
			rows: 10,
			currentPage: 1,
			totalRecords: 0
		})
		const filter = reactive<IEndpointListParams>({
			name: undefined,
			limit: pagination.rows,
			offset: (pagination.currentPage - 1) * pagination.rows,
			order_by: undefined,
			order_dir: undefined
		})
		const endpoints = ref<IEndpoint[]>([])
		const currentEndpoint = ref<IEndpoint | null>()
		const loading = reactive({
			table: false,
			form: false
		})
		const error = ref<any>({})
		const selection = reactive<{ hosts: IHostSelect[]; types: any[] }>({
			hosts: [],
			types: Object.entries(EndpointType).map(([key, value]) => ({
				label: key,
				value: value
			}))
		})

		const fetchEndpoints = async () => {
			loading.table = true
			error.value = null

			try {
				const response = await endpointsApi.getEndpoints(filter)
				if (response?.success) {
					endpoints.value = response.data.endpoints
					pagination.totalRecords = response.data.total ?? 0
				}
			} catch (err: any) {
				error.value = err.message || 'Failed to fetch endpoints'
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

		const createEndpoint = async (payload: IEndpointForm, clusterURLs?: string[]) => {
			loading.form = true
			error.value = null
			try {
				const response = await endpointsApi.createEndpoint(payload, clusterURLs)
				if (response && response.success) {
					await fetchEndpoints()
					return response
				}
			} catch (err: any) {
				error.value = err.message || 'Failed to create endpoint'
				throw err
			} finally {
				loading.form = false
			}
		}

		const updateEndpoint = async (
			endpointId: string,
			payload: IEndpointForm,
			clusterURLs?: string[]
		) => {
			loading.form = true
			error.value = null
			try {
				const response = await endpointsApi.updateEndpoint(endpointId, payload, clusterURLs)
				if (response && response.success) {
					await fetchEndpoints()
					return response
				}
			} catch (err: any) {
				error.value = err.message || 'Failed to update endpoint'
				throw err
			} finally {
				loading.form = false
			}
		}

		const deleteEndpoint = async (endpointId: string, clusterURLs?: string[]) => {
			loading.form = true
			error.value = null
			try {
				const response = await endpointsApi.deleteEndpoint(endpointId, clusterURLs)
				if (response && response.success) {
					endpoints.value = endpoints.value.filter((e) => e.endpoint_id !== endpointId)
					return response
				}
			} catch (err: any) {
				error.value = err.message || 'Failed to delete endpoint'
				throw err
			} finally {
				loading.form = false
			}
		}

		const endpointModalShow = ref<boolean>(false)
		const endpointFormMode = ref<FormMode>(FormMode.ADD)
		const toggleEndpointModalForm = (
			value: boolean,
			formMode: FormMode = FormMode.ADD,
			endpoint: IEndpoint | null = null
		) => {
			endpointModalShow.value = value
			endpointFormMode.value = formMode
			currentEndpoint.value = endpoint
		}

		const setDetailCanvas = async (payload) => {
			endpointFormMode.value = FormMode.UPDATE

			try {
				const response = await endpointsApi.getEndpoints({ name: payload.name })

				if (response?.success) {
					const { endpoints } = response.data
					const data = endpoints.find((h) => h.endpoint_id == payload.endpoint_id) ?? null
					currentEndpoint.value = data
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
			endpoints,
			currentEndpoint,
			loading,
			error,
			selection,
			fetchEndpoints,
			fetchHosts,
			createEndpoint,
			updateEndpoint,
			deleteEndpoint,
			endpointModalShow,
			endpointFormMode,
			toggleEndpointModalForm,
			setDetailCanvas
		}
	},
	{ persist: false }
)

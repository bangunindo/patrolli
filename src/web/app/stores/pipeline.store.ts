// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { defineStore } from 'pinia'
import { computed, reactive, ref } from 'vue'
import pipelineManagementApi from '@/app/api/pipeline.management.api'
import hostsApi from '@/app/api/hosts.api'
import sinksApi from '@/app/api/sinks.api'
import appHelper from '@/app/helpers/app.helper'
import { FormMode } from '@@/shared/types/app.type'
import type {
	CreatePipelineManagementRequest,
	IHostSelectorItem,
	IPipelineCreateForm,
	IPipelineItem,
	IPipelineManagementQueryParams,
	ISinkSelectorItem,
	PipelineAccess,
	PipelineManagementDetail,
	PipelineStatus,
	UpdatePipelineManagementRequest
} from '@@/shared/types/pipeline.management.type'

export const usePipelineStore = defineStore(
	'pipeline',
	() => {
		const pagination = reactive({
			rows: 10,
			currentPage: 1,
			totalRecords: 0
		})

		const filter = reactive<IPipelineManagementQueryParams>({
			name: undefined,
			limit: pagination.rows,
			offset: (pagination.currentPage - 1) * pagination.rows,
			order_by: undefined,
			order_dir: undefined
		})

		const loading = reactive({
			table: false,
			form: false,
			resources: false
		})

		const error = ref<string | null>(null)

		const pipelines = ref<IPipelineItem[]>([])
		const currentPipeline = ref<IPipelineItem | null>(null)

		const availableHosts = ref<IHostSelectorItem[]>([])
		const availableSinks = ref<ISinkSelectorItem[]>([])

		const pipelineModalShow = ref<boolean>(false)
		const pipelineFormMode = ref<FormMode>(FormMode.ADD)

		const form = reactive<IPipelineCreateForm>({
			name: '',
			cluster_url: appHelper.getDefaultClusterValue() || '',
			access: 'public',
			visibility: 1,
			adm_status: 1,
			accessibleUsers: [],
			assignedHosts: [],
			assignedSinks: []
		})

		function resetForm() {
			form.name = ''
			form.cluster_url = appHelper.getDefaultClusterValue() || ''
			form.access = 'public'
			form.visibility = 1
			form.adm_status = 1
			form.accessibleUsers = []
			form.assignedHosts = []
			form.assignedSinks = []
			form.last_updated = undefined
		}

		const fetchPipelines = async (clusterURLs?: string[]) => {
			loading.table = true
			error.value = null
			try {
				const response = await pipelineManagementApi.getPipelines(filter, clusterURLs)
				if (response?.success && response.data) {
					const items = response.data.pipeline_managements ?? []
					pipelines.value = items.map((item: any) => {
						const status: PipelineStatus = item.adm_status === 1 ? 'running' : 'stopped'
						const access: PipelineAccess = item.visibility === 1 ? 'public' : 'private'
						const hostIds: string[] = item.host_ids || []
						const sinkIds: string[] = item.sink_ids || []
						const clusterUrl = item.cluster_url || ''
						return {
							id: item.pipeline_management_id,
							name: item.name,
							code: `pipe-${item.pipeline_management_id.slice(0, 8)}`,
							status,
							adm_status: item.adm_status,
							access,
							visibility: item.visibility,
							assignedHostsCount: item.host_count ?? hostIds.length,
							assignedSinksCount: item.sink_count ?? sinkIds.length,
							lastUpdate: item.last_updated
								? new Date(item.last_updated).toLocaleString()
								: 'Just now',
							assignedHosts: hostIds,
							assignedSinks: sinkIds,
							createdAt: item.created_at,
							cluster_url: clusterUrl,
							cluster_name:
								item.cluster_name ||
								(clusterUrl ? appHelper.getClusterName(clusterUrl) : ''),
							raw: item
						} as IPipelineItem
					})
					pagination.totalRecords = response.data.total ?? 0
				}
			} catch (err: any) {
				error.value = err.message || 'Failed to fetch pipelines'
			} finally {
				loading.table = false
			}
		}

		// Fetch hosts for selector popover using hostsApi
		async function fetchHostsForCluster(clusterURL?: string) {
			loading.resources = true
			try {
				const cUrls = clusterURL ? [clusterURL] : undefined
				const res = await hostsApi.getHosts({ limit: 100 }, cUrls)
				if (res?.success && res.data?.hosts) {
					availableHosts.value = res.data.hosts.map((h: any) => ({
						id: h.host_id,
						name: h.name,
						ip: h.ip || h.host_id,
						environment: (h.environment as any) || 'prod'
					}))
				} else {
					availableHosts.value = []
				}
			} catch {
				availableHosts.value = []
			} finally {
				loading.resources = false
			}
		}

		// Fetch sinks for selector popover using sinksApi
		async function fetchSinksForCluster(clusterURL?: string) {
			loading.resources = true
			try {
				const cUrls = clusterURL ? [clusterURL] : undefined
				const res = await sinksApi.getSinks({ limit: 100 }, cUrls)
				if (res?.success && res.data?.sinks) {
					availableSinks.value = res.data.sinks.map((s: any) => ({
						id: s.sink_id,
						name: s.name,
						type: s.type || 'sink'
					}))
				} else {
					availableSinks.value = []
				}
			} catch {
				availableSinks.value = []
			} finally {
				loading.resources = false
			}
		}

		async function openPipelineDetail(id: string, clusterURL?: string) {
			try {
				const cUrls = clusterURL ? [clusterURL] : undefined
				const res = await pipelineManagementApi.getPipelineById(id, cUrls)
				if (res?.success && res.data) {
					return res.data as PipelineManagementDetail
				}
			} catch (err: any) {
				error.value = err.message || 'Failed to load pipeline detail'
			}
			return null
		}

		const togglePipelineModalForm = async (
			value: boolean,
			formMode: FormMode = FormMode.ADD,
			pipeline: IPipelineItem | null = null
		) => {
			pipelineModalShow.value = value
			pipelineFormMode.value = formMode
			currentPipeline.value = pipeline

			if (value && pipeline && formMode === FormMode.UPDATE) {
				form.name = pipeline.name
				form.cluster_url = pipeline.cluster_url || appHelper.getDefaultClusterValue() || ''
				form.access = pipeline.access
				form.visibility = pipeline.visibility ?? (pipeline.access === 'public' ? 1 : 0)
				form.adm_status = pipeline.adm_status ?? 1
				form.accessibleUsers = [...(pipeline.accessibleUsers || [])]
				form.assignedHosts = [...pipeline.assignedHosts]
				form.assignedSinks = [...pipeline.assignedSinks]

				// Load fresh resource lists for cluster
				await Promise.all([
					fetchHostsForCluster(form.cluster_url),
					fetchSinksForCluster(form.cluster_url)
				])

				// Load full detail (host_ids, sink_ids, last_updated)
				const detail = await openPipelineDetail(pipeline.id, form.cluster_url)
				if (detail) {
					form.last_updated = detail.last_updated
					form.assignedHosts = [...(detail.host_ids || [])]
					form.assignedSinks = [...(detail.sink_ids || [])]
					form.visibility = detail.visibility
					form.access = detail.visibility === 1 ? 'public' : 'private'
					form.adm_status = detail.adm_status
				}
			} else if (!value) {
				resetForm()
			} else if (value && formMode === FormMode.ADD) {
				resetForm()
				await Promise.all([
					fetchHostsForCluster(form.cluster_url),
					fetchSinksForCluster(form.cluster_url)
				])
			}
		}

		const isStep1Completed = computed(() => form.name.trim().length > 0)
		const isStep2Completed = computed(() => form.assignedHosts.length > 0)
		const isStep3Completed = computed(() => form.assignedSinks.length > 0)

		const completedStepsCount = computed(() => {
			let count = 0
			if (isStep1Completed.value) count++
			if (isStep2Completed.value) count++
			if (isStep3Completed.value) count++
			return count
		})

		async function savePipeline(): Promise<boolean> {
			if (!form.name.trim()) return false
			if (form.assignedHosts.length === 0) return false

			loading.form = true
			error.value = null

			try {
				const cUrls = form.cluster_url ? [form.cluster_url] : undefined
				const visibility = form.access === 'public' ? 1 : 0

				if (pipelineFormMode.value === FormMode.UPDATE && currentPipeline.value) {
					const payload: UpdatePipelineManagementRequest = {
						name: form.name.trim(),
						adm_status: form.adm_status ?? 1,
						visibility,
						host_ids: form.assignedHosts,
						sink_ids: form.assignedSinks,
						last_updated: form.last_updated || new Date().toISOString(),
						force: true
					}

					const res = await pipelineManagementApi.updatePipeline(
						currentPipeline.value.id,
						payload,
						cUrls
					)
					if (res?.success) {
						await fetchPipelines()
						togglePipelineModalForm(false)
						return true
					}
					return false
				}

				const payload: CreatePipelineManagementRequest = {
					name: form.name.trim(),
					adm_status: 1,
					visibility,
					host_ids: form.assignedHosts,
					sink_ids: form.assignedSinks
				}

				const res = await pipelineManagementApi.createPipeline(payload, cUrls)
				if (res?.success) {
					await fetchPipelines()
					togglePipelineModalForm(false)
					return true
				}
				return false
			} catch (err: any) {
				error.value = err.message || 'Failed to save pipeline'
				return false
			} finally {
				loading.form = false
			}
		}

		async function deletePipeline(id: string, clusterURL?: string): Promise<boolean> {
			try {
				const cUrls = clusterURL ? [clusterURL] : undefined
				await pipelineManagementApi.deletePipeline(id, cUrls)
				pipelines.value = pipelines.value.filter((p) => p.id !== id)
				return true
			} catch (err: any) {
				error.value = err.message || 'Failed to delete pipeline'
				return false
			}
		}

		async function togglePipelineStatus(id: string, clusterURL?: string) {
			const target = pipelines.value.find((p) => p.id === id)
			if (!target) return

			const nextAdmStatus = target.status === 'running' ? 0 : 1
			try {
				const cUrls =
					clusterURL || target.cluster_url
						? [clusterURL || target.cluster_url!]
						: undefined
				const detail = await openPipelineDetail(id, clusterURL || target.cluster_url)
				if (detail) {
					const payload: UpdatePipelineManagementRequest = {
						name: detail.name,
						adm_status: nextAdmStatus,
						visibility: detail.visibility,
						host_ids: detail.host_ids || [],
						sink_ids: detail.sink_ids || [],
						last_updated: detail.last_updated,
						force: true
					}
					const res = await pipelineManagementApi.updatePipeline(id, payload, cUrls)
					if (res?.success) {
						target.status = nextAdmStatus === 1 ? 'running' : 'stopped'
						target.adm_status = nextAdmStatus
					}
				}
			} catch (err: any) {
				error.value = err.message || 'Failed to update pipeline status'
			}
		}

		async function startSelected(ids: string[] = []) {
			await Promise.all(
				ids.map(async (id) => {
					const target = pipelines.value.find((p) => p.id === id)
					if (target && target.status !== 'running') {
						await togglePipelineStatus(id, target.cluster_url)
					}
				})
			)
		}

		async function stopSelected(ids: string[] = []) {
			await Promise.all(
				ids.map(async (id) => {
					const target = pipelines.value.find((p) => p.id === id)
					if (target && target.status === 'running') {
						await togglePipelineStatus(id, target.cluster_url)
					}
				})
			)
		}

		async function deleteSelected(ids: string[] = []) {
			await Promise.all(
				ids.map(async (id) => {
					const target = pipelines.value.find((p) => p.id === id)
					await deletePipeline(id, target?.cluster_url)
				})
			)
		}

		return {
			filter,
			pagination,
			pipelines,
			currentPipeline,
			loading,
			error,
			availableHosts,
			availableSinks,
			startSelected,
			stopSelected,
			deleteSelected,
			togglePipelineStatus,
			deletePipeline,
			fetchPipelines,
			fetchHostsForCluster,
			fetchSinksForCluster,
			openPipelineDetail,
			pipelineModalShow,
			pipelineFormMode,
			togglePipelineModalForm,
			form,
			isStep1Completed,
			isStep2Completed,
			isStep3Completed,
			completedStepsCount,
			resetForm,
			savePipeline
		}
	},
	{ persist: false }
)

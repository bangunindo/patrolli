// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { defineStore } from 'pinia'
import { computed, reactive, ref } from 'vue'
import pipelinesApi from '@/app/api/pipelines.api'
import appHelper from '@/app/helpers/app.helper'
import {
	createHostsPaginatedResource,
	createSinksPaginatedResource
} from '@/app/helpers/pipeline-resource.helper'
import { FormMode } from '@@/shared/types/app.type'
import type {
	CreatePipelineRequest,
	IHostSelectorItem,
	IPipelineCreateForm,
	IPipelineItem,
	IPipelineQueryParams,
	ISinkSelectorItem,
	PipelineAccess,
	PipelineDetail,
	PipelineStatus,
	UpdatePipelineRequest
} from '@@/shared/types/pipeline.type'

export const usePipelineStore = defineStore(
	'pipeline',
	() => {
		const pagination = reactive({
			rows: 10,
			currentPage: 1,
			totalRecords: 0
		})

		const filter = reactive<IPipelineQueryParams>({
			name: undefined,
			limit: pagination.rows,
			offset: (pagination.currentPage - 1) * pagination.rows,
			order_by: undefined,
			order_dir: undefined
		})

		const loading = reactive({
			table: false,
			form: false,
			hosts: false,
			hostsMore: false,
			sinks: false,
			sinksMore: false
		})

		const error = ref<string | null>(null)

		const pipelines = ref<IPipelineItem[]>([])
		const currentPipeline = ref<IPipelineItem | null>(null)

		function resolveClusterUrls(clusterURL?: string): string[] | undefined {
			const target = clusterURL !== undefined ? clusterURL : form.cluster_url
			return target ? [target] : undefined
		}

		const hostsResource = createHostsPaginatedResource<IHostSelectorItem>({
			mapItem: (h) => ({
				id: h.host_id,
				name: h.name,
				ip: h.ip || h.host_id,
				environment: (h.environment as any) || 'prod'
			}),
			getId: (h) => h.id,
			clearOnError: true,
			onLoadingChange: (isLoading, isMore) => {
				loading.hosts = isLoading
				loading.hostsMore = isMore
			}
		})

		const sinksResource = createSinksPaginatedResource<ISinkSelectorItem>({
			mapItem: (s) => ({
				id: s.sink_id,
				name: s.name,
				type: s.type || 'sink'
			}),
			getId: (s) => s.id,
			clearOnError: true,
			onLoadingChange: (isLoading, isMore) => {
				loading.sinks = isLoading
				loading.sinksMore = isMore
			}
		})

		function buildNameMap<T extends { name?: string }>(
			cache: Record<string, T>
		): Record<string, string> {
			const map: Record<string, string> = {}
			for (const [id, item] of Object.entries(cache)) {
				if (item?.name) map[id] = item.name
			}
			return map
		}

		const availableHosts = hostsResource.items
		const availableHostsTotal = hostsResource.total
		const hostsSearchQuery = hostsResource.searchQuery
		const hostNameCache = computed(() => buildNameMap(hostsResource.cache.value))

		const availableSinks = sinksResource.items
		const availableSinksTotal = sinksResource.total
		const sinksSearchQuery = sinksResource.searchQuery
		const sinkNameCache = computed(() => buildNameMap(sinksResource.cache.value))

		const FORM_VISIBLE_TAG_HYDRATION_LIMIT = 50

		function hydrateHostsByIds(ids: string[], clusterURL?: string) {
			return hostsResource.hydrateByIds(ids, resolveClusterUrls(clusterURL))
		}

		function hydrateSinksByIds(ids: string[], clusterURL?: string) {
			return sinksResource.hydrateByIds(ids, resolveClusterUrls(clusterURL))
		}

		function resetSelectorCaches() {
			hostsResource.reset()
			sinksResource.reset()
		}

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
			resetSelectorCaches()
		}

		const resetFilter = () => {
			filter.name = undefined
			filter.order_by = undefined
			filter.order_dir = undefined
			filter.offset = 0
			pagination.currentPage = 1
		}

		const fetchPipelines = async (clusterURLs?: string[]) => {
			loading.table = true
			error.value = null
			try {
				const response = await pipelinesApi.getPipelines(filter, clusterURLs)
				if (response?.success && response.data) {
					const items = response.data.pipelines ?? []
					pipelines.value = items.map((item: any) => {
						const status: PipelineStatus = item.adm_status === 1 ? 'running' : 'stopped'
						const access: PipelineAccess = item.visibility === 1 ? 'public' : 'private'
						const hostIds: string[] = item.host_ids || []
						const sinkIds: string[] = item.sink_ids || []
						const clusterUrl = item.cluster_url || ''
						return {
							id: item.pipeline_id,
							name: item.name,
							code: `pipe-${item.pipeline_id.slice(0, 8)}`,
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

		function fetchHostsForCluster(clusterURL?: string, search = '') {
			return hostsResource.fetchFirst(resolveClusterUrls(clusterURL), search)
		}

		function loadMoreHosts(clusterURL?: string) {
			return hostsResource.loadMore(resolveClusterUrls(clusterURL))
		}

		function resolveAllHostIds(clusterURL?: string, search = ''): Promise<string[]> {
			return hostsResource.resolveAllIds(resolveClusterUrls(clusterURL), search)
		}

		function fetchSinksForCluster(clusterURL?: string, search = '') {
			return sinksResource.fetchFirst(resolveClusterUrls(clusterURL), search)
		}

		function loadMoreSinks(clusterURL?: string) {
			return sinksResource.loadMore(resolveClusterUrls(clusterURL))
		}

		function resolveAllSinkIds(clusterURL?: string, search = ''): Promise<string[]> {
			return sinksResource.resolveAllIds(resolveClusterUrls(clusterURL), search)
		}

		async function openPipelineDetail(id: string, clusterURL?: string) {
			try {
				const cUrls = clusterURL ? [clusterURL] : undefined
				const res = await pipelinesApi.getPipelineById(id, cUrls)
				if (res?.success && res.data) {
					return res.data as PipelineDetail
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
				resetSelectorCaches()
				form.name = pipeline.name
				form.cluster_url = pipeline.cluster_url || appHelper.getDefaultClusterValue() || ''
				form.access = pipeline.access
				form.visibility = pipeline.visibility ?? (pipeline.access === 'public' ? 1 : 0)
				form.adm_status = pipeline.adm_status ?? 1
				form.accessibleUsers = [...(pipeline.accessibleUsers || [])]
				form.assignedHosts = [...pipeline.assignedHosts]
				form.assignedSinks = [...pipeline.assignedSinks]

				const [, , detail] = await Promise.all([
					fetchHostsForCluster(form.cluster_url),
					fetchSinksForCluster(form.cluster_url),
					openPipelineDetail(pipeline.id, form.cluster_url)
				])

				if (detail) {
					form.last_updated = detail.last_updated
					form.assignedHosts = [...(detail.host_ids || [])]
					form.assignedSinks = [...(detail.sink_ids || [])]
					form.visibility = detail.visibility
					form.access = detail.visibility === 1 ? 'public' : 'private'
					form.adm_status = detail.adm_status

					await Promise.all([
						hydrateHostsByIds(
							form.assignedHosts.slice(0, FORM_VISIBLE_TAG_HYDRATION_LIMIT),
							form.cluster_url
						),
						hydrateSinksByIds(
							form.assignedSinks.slice(0, FORM_VISIBLE_TAG_HYDRATION_LIMIT),
							form.cluster_url
						)
					])
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
					const payload: UpdatePipelineRequest = {
						name: form.name.trim(),
						adm_status: form.adm_status ?? 1,
						visibility,
						host_ids: form.assignedHosts,
						sink_ids: form.assignedSinks,
						last_updated: form.last_updated || new Date().toISOString(),
						force: true
					}

					const res = await pipelinesApi.updatePipeline(
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

				const payload: CreatePipelineRequest = {
					name: form.name.trim(),
					adm_status: 1,
					visibility,
					host_ids: form.assignedHosts,
					sink_ids: form.assignedSinks
				}

				const res = await pipelinesApi.createPipeline(payload, cUrls)
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
				await pipelinesApi.deletePipeline(id, cUrls)
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
					const payload: UpdatePipelineRequest = {
						name: detail.name,
						adm_status: nextAdmStatus,
						visibility: detail.visibility,
						host_ids: detail.host_ids || [],
						sink_ids: detail.sink_ids || [],
						last_updated: detail.last_updated,
						force: true
					}
					const res = await pipelinesApi.updatePipeline(id, payload, cUrls)
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
			availableHostsTotal,
			hostsSearchQuery,
			hostNameCache,
			availableSinks,
			availableSinksTotal,
			sinksSearchQuery,
			sinkNameCache,
			startSelected,
			stopSelected,
			deleteSelected,
			togglePipelineStatus,
			deletePipeline,
			resetFilter,
			fetchPipelines,
			fetchHostsForCluster,
			loadMoreHosts,
			resolveAllHostIds,
			hydrateHostsByIds,
			fetchSinksForCluster,
			loadMoreSinks,
			resolveAllSinkIds,
			hydrateSinksByIds,
			resetSelectorCaches,
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

// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { reactive, ref } from 'vue'

import { defineStore } from 'pinia'

import hostsApi from '@/app/api/hosts.api'

import nodesApi from '@/app/api/nodes.api'
import templatesApi from '@/app/api/templates.api'
import { FormMode } from '@@/shared/types/app.type'
import type { IClusterNodeSelect } from '@@/shared/types/cluster.type'
import type {
	IHost,
	IHostForm,
	IHostFormTemplate,
	IHostListParams
} from '@@/shared/types/host.type'
import type {
	ITemplateApplyForm,
	ITemplateSelect,
	ITemplateUnapplyForm
} from '@@/shared/types/template.type'

export const useHostsStore = defineStore(
	'hosts',
	() => {
		const pagination = reactive({
			rows: 10,
			currentPage: 1,
			totalRecords: 0
		})
		const filter = reactive<IHostListParams>({
			name: undefined,
			monitored_by: undefined,
			limit: pagination.rows,
			offset: (pagination.currentPage - 1) * pagination.rows,
			order_by: undefined,
			order_dir: undefined
		})

		const hosts = ref<IHost[]>([])
		const currentHost = ref<IHost | null>(null)
		const loading = reactive({
			table: false,
			form: false
		})
		const error = ref<string | null>(null)
		const selection = reactive<{ templates: ITemplateSelect[]; nodes: IClusterNodeSelect[] }>({
			templates: [],
			nodes: []
		})

		const fetchHosts = async () => {
			loading.table = true
			error.value = null
			try {
				const response = await hostsApi.getHosts(filter)
				if (response?.success) {
					hosts.value = response.data.hosts
					pagination.totalRecords = response.data.total ?? 0
				}
			} catch (err: any) {
				error.value = err.message || 'Failed to fetch hosts'
			} finally {
				loading.table = false
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
				error.value = err.message || 'Failed to fetch hosts'
			} finally {
				loading.form = false
			}
		}

		const fetchNodes = async () => {
			loading.form = true
			error.value = null
			try {
				const response = await nodesApi.getNodes()
				if (response?.success) {
					selection.nodes = response.data.nodes.map((node) => ({
						...node,
						value: node.node_id,
						label: node.name
					}))
				}
			} catch (err: any) {
				error.value = err.message || 'Failed to fetch hosts'
			} finally {
				loading.form = false
			}
		}

		const createHost = async (
			payload: IHostForm,
			template: IHostFormTemplate,
			clusterURLs?: string[]
		) => {
			loading.form = true
			error.value = null
			try {
				const response = await hostsApi.createHost(payload, clusterURLs)
				if (response && response?.success) {
					if (
						template.template_id &&
						template.template_id.length > 0 &&
						response.data?.host_id
					) {
						const hostId = response.data.host_id
						await linkHostsToTemplates([hostId], template, clusterURLs)
					}

					await fetchHosts() // Refresh the list
					return response
				}
			} catch (err: any) {
				error.value = err.message || 'Failed to create host'
				throw err
			} finally {
				loading.form = false
			}
		}

		const unlinkHostsFromTemplates = async (
			hostIds: string[],
			templateChoices: { template_id: string; keep_items: boolean }[],
			clusterURLs?: string[]
		) => {
			try {
				const unapplyList: any[] = []
				hostIds.forEach((hostid) => {
					templateChoices.forEach((choice) => {
						const unapply = {
							host_id: hostid,
							keep_items: choice.keep_items,
							template_id: choice.template_id
						}

						unapplyList.push(unapply)
					})
				})

				const body: ITemplateUnapplyForm = {
					unapply_list: unapplyList
				}

				const responses = await templatesApi.unapplyTemplate(body, clusterURLs)
				return responses
			} catch (err: any) {
				error.value = err.message || 'Failed to unapply template'
				throw err
			}
		}

		const updateHost = async (
			id: string,
			payload: IHostForm,
			template: IHostFormTemplate,
			unapplyChoices: { template_id: string; keep_items: boolean }[] = [],
			clusterURLs?: string[]
		) => {
			loading.form = true
			error.value = null
			try {
				const response = await hostsApi.updateHost(id, payload, clusterURLs)
				if (response && response.success) {
					const existingTemplateIds =
						currentHost.value?.templates?.map((t) => t.template_id) || []
					const newTemplateIds = template.template_id || []

					const toApply = newTemplateIds.filter(
						(tid) => !existingTemplateIds.includes(tid)
					)

					if (toApply.length > 0) {
						await linkHostsToTemplates([id], { template_id: toApply }, clusterURLs)
					}

					if (unapplyChoices.length > 0) {
						await unlinkHostsFromTemplates([id], unapplyChoices, clusterURLs)
					}

					await fetchHosts() // Refresh the list
					return response
				}
			} catch (err: any) {
				error.value = err.message || 'Failed to update host'
				throw err
			} finally {
				loading.form = false
			}
		}

		const deleteHost = async (hostId: string, clusterURLs?: string[]) => {
			loading.form = true
			error.value = null
			try {
				const response = await hostsApi.deleteHost(hostId, clusterURLs)
				if (response && response.success) {
					hosts.value = hosts.value.filter((h) => h.host_id !== hostId)
					return response
				}
			} catch (err: any) {
				error.value = err.message || 'Failed to delete host'
				throw err
			} finally {
				loading.form = false
			}
		}

		const linkHostsToTemplates = async (
			hostIds: string[],
			payload: IHostFormTemplate,
			clusterURLs?: string[]
		) => {
			loading.form = true
			error.value = null
			try {
				const body: ITemplateApplyForm = {
					host_ids: hostIds,
					template_ids: payload.template_id ?? []
				}
				const responses = await templatesApi.applyTemplate(body, clusterURLs)

				if (responses) {
					await fetchHosts()
					return responses
				}
			} catch (err: any) {
				error.value = err.message || 'Failed to create host'
				throw err
			} finally {
				loading.form = false
			}
		}

		const hostModalShow = ref<boolean>(false)
		const hostFormMode = ref<FormMode>(FormMode.ADD)
		const toggleHostModalForm = (
			value: boolean,
			formMode: FormMode = FormMode.ADD,
			host: IHost | null = null
		) => {
			hostModalShow.value = value
			hostFormMode.value = formMode
			currentHost.value = host
		}

		const setDetailCanvas = async (payload) => {
			hostFormMode.value = FormMode.UPDATE

			try {
				const response = await hostsApi.getHosts({ name: payload.name })

				if (response?.success) {
					const { hosts } = response.data
					const host = hosts.find((h) => h.host_id == payload.host_id) ?? null
					currentHost.value = host
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
			hosts,
			currentHost,
			loading,
			error,
			fetchHosts,
			createHost,
			updateHost,
			deleteHost,
			linkHostsToTemplates,
			unlinkHostsFromTemplates,
			hostModalShow,
			hostFormMode,
			toggleHostModalForm,
			selection,
			fetchTemplates,
			fetchNodes,
			setDetailCanvas
		}
	},
	{ persist: false }
)

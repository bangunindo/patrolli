// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { reactive, ref } from 'vue'

import { defineStore } from 'pinia'

import templatesApi from '@/app/api/templates.api'

import { FormMode } from '@@/shared/types/app.type'
import type {
	ITemplate,
	ITemplateForm,
	ITemplateLinkToPayload,
	ITemplateListParams
} from '@@/shared/types/template.type'

export const useTemplatesStore = defineStore(
	'templates',
	() => {
		const pagination = reactive({
			rows: 10,
			currentPage: 1,
			totalRecords: 0
		})
		const filter = reactive<ITemplateListParams>({
			name: undefined,
			limit: pagination.rows,
			offset: (pagination.currentPage - 1) * pagination.rows,
			order_by: undefined,
			order_dir: undefined
		})
		const templates = ref<ITemplate[]>([])
		const currentTemplate = ref<ITemplate | null>(null)
		const loading = reactive({
			table: false,
			form: false
		})
		const error = ref<string | null>(null)

		const fetchTemplates = async (clusterURLs?: string[]) => {
			loading.table = true
			error.value = null

			try {
				const response = await templatesApi.getTemplates(filter, clusterURLs)
				if (response?.success) {
					templates.value = response.data.templates
					pagination.totalRecords = response.data.total ?? 0
				}
			} catch (err: any) {
				error.value = err.message || 'Failed to fetch templates'
			} finally {
				loading.table = false
			}
		}

		const createTemplate = async (payload: ITemplateForm, clusterURLs?: string[]) => {
			loading.form = true
			error.value = null
			try {
				const response = await templatesApi.createTemplate(payload, clusterURLs)
				if (response && response.success) {
					await fetchTemplates()
					return response
				}
			} catch (err: any) {
				error.value = err.message || 'Failed to create template'
				throw err
			} finally {
				loading.form = false
			}
		}

		const updateTemplateLink = async (
			templateId: string,
			payload: ITemplateLinkToPayload,
			clusterURLs?: string[]
		) => {
			loading.form = true
			error.value = null
			try {
				const response = await templatesApi.linkTemplate(templateId, payload, clusterURLs)
				if (response && response.success) {
					await fetchTemplates()
					return response
				}
			} catch (err: any) {
				error.value = err.message || 'Failed to update templates'
				throw err
			} finally {
				loading.form = false
			}
		}

		const linkTemplatesFor = async (
			templateId: string,
			linkToIds: string[],
			clusterURLs?: string[]
		) => {
			loading.form = true
			error.value = null
			try {
				const promises = linkToIds.map((linkId) =>
					templatesApi.linkTemplate(
						templateId,
						{ link_to_template_id: linkId },
						clusterURLs
					)
				)
				await Promise.all(promises)
			} catch (err: any) {
				error.value = err.message || 'Failed to link templates'
				throw err
			} finally {
				loading.form = false
			}
		}

		const unlinkTemplatesFor = async (
			templateId: string,
			unlinkChoices: { unlink_from_template_id: string; keep_items: boolean }[],
			clusterURLs?: string[]
		) => {
			loading.form = true
			error.value = null
			try {
				const promises = unlinkChoices.map((choice) =>
					templatesApi.unlinkTemplate(templateId, choice, clusterURLs)
				)
				await Promise.all(promises)
			} catch (err: any) {
				error.value = err.message || 'Failed to unlink templates'
				throw err
			} finally {
				loading.form = false
			}
		}

		const fetchTemplatesDetail = async (templateId: string, clusterURLs?: string[]) => {
			loading.form = true
			error.value = null
			try {
				const response = await templatesApi.getTemplateById(templateId, clusterURLs)
				if (response?.success) {
					return response.data
				}
			} catch (err: any) {
				error.value = err.message || 'Failed to fetch templates'
			} finally {
				loading.form = false
			}
		}

		const updateTemplate = async (
			templateId: string,
			payload: ITemplateForm,
			clusterURLs?: string[]
		) => {
			loading.form = true
			error.value = null
			try {
				const response = await templatesApi.updateTemplate(templateId, payload, clusterURLs)
				if (response && response.success) {
					await fetchTemplates()
					return response
				}
			} catch (err: any) {
				error.value = err.message || 'Failed to update template'
				throw err
			} finally {
				loading.form = false
			}
		}

		const deleteTemplate = async (templateId: string, clusterURLs?: string[]) => {
			loading.form = true
			error.value = null
			try {
				const response = await templatesApi.deleteTemplate(templateId, clusterURLs)
				if (response && response.success) {
					templates.value = templates.value.filter((t) => t.template_id !== templateId)
					return response
				}
			} catch (err: any) {
				error.value = err.message || 'Failed to delete template'
				throw err
			} finally {
				loading.form = false
			}
		}

		const templateModalShow = ref<boolean>(false)
		const templateFormMode = ref<FormMode>(FormMode.ADD)
		const toggleTemplateModalForm = (
			value: boolean,
			formMode: FormMode = FormMode.ADD,
			template: ITemplate | null = null
		) => {
			templateModalShow.value = value
			templateFormMode.value = formMode
			currentTemplate.value = template
		}

		return {
			filter,
			pagination,
			templates,
			currentTemplate,
			loading,
			error,
			fetchTemplates,
			createTemplate,
			updateTemplateLink,
			linkTemplatesFor,
			unlinkTemplatesFor,
			fetchTemplatesDetail,
			updateTemplate,
			deleteTemplate,
			templateModalShow,
			templateFormMode,
			toggleTemplateModalForm
		}
	},
	{ persist: false }
)

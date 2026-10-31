// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type {
	ITemplateApplyForm,
	ITemplateForm,
	ITemplateLinkToPayload,
	ITemplateListParams,
	ITemplateListResponse,
	ITemplateResponse,
	ITemplateUnapplyForm
} from '@@/shared/types/template.type'
import { httpService } from '@/app/services/http.service'

const templatesApi = {
	path: '/api/templates',

	async getTemplates(
		query?: ITemplateListParams,
		clusterURLs?: string[]
	): Promise<ITemplateListResponse | undefined> {
		return httpService.get(
			this.path,
			{
				params: query ?? {}
			},
			clusterURLs
		)
	},

	async createTemplate(
		payload: ITemplateForm,
		clusterURLs?: string[]
	): Promise<ITemplateResponse | undefined> {
		return httpService.post(this.path, payload, undefined, clusterURLs)
	},

	async linkTemplate(
		templateId: string,
		payload?: ITemplateLinkToPayload,
		clusterURLs?: string[]
	): Promise<ITemplateResponse | undefined> {
		return httpService.put(`${this.path}/link/${templateId}`, payload, undefined, clusterURLs)
	},

	async unlinkTemplate(
		templateId: string,
		payload: { unlink_from_template_id: string },
		clusterURLs?: string[]
	): Promise<ITemplateResponse | undefined> {
		return httpService.put(`${this.path}/unlink/${templateId}`, payload, undefined, clusterURLs)
	},

	async getTemplateById(
		templateId: string,
		clusterURLs?: string[]
	): Promise<ITemplateResponse | undefined> {
		return httpService.get(`${this.path}/${templateId}`, undefined, clusterURLs)
	},

	async updateTemplate(
		templateId: string,
		payload: ITemplateForm,
		clusterURLs?: string[]
	): Promise<ITemplateResponse | undefined> {
		return httpService.put(`${this.path}/${templateId}`, payload, undefined, clusterURLs)
	},

	async deleteTemplate(
		templateId: string,
		clusterURLs?: string[]
	): Promise<ITemplateResponse | undefined> {
		try {
			return await httpService.delete(`${this.path}/${templateId}`, undefined, clusterURLs)
		} catch (error: any) {
			throw new Error(error.message || 'Failed to delete template')
		}
	},

	async applyTemplate(
		payload: ITemplateApplyForm,
		clusterURLs?: string[]
	): Promise<null | undefined> {
		const url = `${this.path}/apply`
		return httpService.post(url, payload, undefined, clusterURLs)
	},

	async unapplyTemplate(
		payload: ITemplateUnapplyForm,
		clusterURLs?: string[]
	): Promise<null | undefined> {
		const url = `${this.path}/unapply`
		return httpService.post(url, payload, undefined, clusterURLs)
	}
}

export default templatesApi

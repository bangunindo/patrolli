// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type {
	BulkApplyTemplateBody,
	BulkUnapplyTemplateBody
} from '@@/shared/generated/templates.type'
import type z from 'zod'
import type { IBaseResponse, IFilterParam } from '@@/shared/types/api.type'

export type TTemplateType = 'snmp_v1' | 'snmp_v2c' | 'snmp_v3'

export interface ITemplate {
	labels: string
	links_to_templates?: { name: string; template_id: string }[]
	name: string
	template_id: string
	item_count?: number
	cluster_url?: string
	cluster_name?: string
}

export interface ITemplateListParams extends IFilterParam {
	name?: string
	order_by?: 'template_id' | 'name'
}

export interface ITemplateListResponse extends IBaseResponse {
	data: {
		templates: ITemplate[]
		total: number
	}
}

export interface ITemplateForm {
	labels: string
	name: string
}

export interface ITemplateLinkToPayload {
	link_to_template_id: string
}

export interface ITemplateResponse extends IBaseResponse {
	data?: ITemplate
}

export interface ITemplateSelect extends ITemplate {
	label?: string
	value?: string
}

export interface ITemplateApplyForm extends z.infer<typeof BulkApplyTemplateBody> {}

export interface ITemplateUnapplyForm extends z.infer<typeof BulkUnapplyTemplateBody> {}

// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { ListHostsQueryParams } from '@@/shared/generated/hosts.type'
import type {
	CreateHostRequest,
	ResponseHostTemplate
} from '@@/shared/generated/patrolliAPI.schemas.type'
import type z from 'zod'
import type { IBaseResponse, IFilterParam, TAdmStatus } from '@@/shared/types/api.type'

export interface IHost {
	host_template_id?: string
	host_id: string
	name: string
	adm_status: TAdmStatus
	labels: any
	last_updated: string
	monitored_by?: string
	templates?: ResponseHostTemplate[]
	item_count?: number
	cluster_url?: string
	cluster_name?: string
}

export interface IHostListParams
	extends Partial<z.infer<typeof ListHostsQueryParams>>, IFilterParam {
	name?: string
	order_by?: 'host_id' | 'name'
}

export interface IHostListResponse extends IBaseResponse {
	data: {
		hosts: IHost[]
		total: number
	}
}

export interface IHostForm extends z.infer<typeof CreateHostRequest> {
	labels: string
}

export interface IHostFormTemplate {
	template_id?: string[]
}

export interface IHostFormTemplatePayload {
	template_id?: string
}

export interface IHostResponse extends IBaseResponse {
	data?: IHost
}

export interface IHostSelect extends IHost {
	label: string
	value: string
}

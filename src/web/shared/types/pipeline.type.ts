// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type {
	CreatePipelineRequest,
	ListPipelineResponseInner,
	PipelineDetail,
	UpdatePipelineRequest
} from '@@/shared/generated/patrolliAPI.schemas.type'
import type {
	GetPipelineParams,
	ListPipelinesQueryParams
} from '@@/shared/generated/pipelines.type'
import type z from 'zod'

export type PipelineStatus = 'running' | 'stopped' | 'draft'
export type PipelineAccess = 'public' | 'private'

export type IPipelineQueryParams = z.infer<typeof ListPipelinesQueryParams>
export type IGetPipelineParams = z.infer<typeof GetPipelineParams>

export interface IPipelineItem {
	id: string
	name: string
	code: string
	status: PipelineStatus
	adm_status?: number
	access: PipelineAccess
	visibility?: number
	assignedHostsCount: number
	assignedSinksCount: number
	lastUpdate: string
	assignedHosts: string[]
	assignedSinks: string[]
	accessibleUsers?: string[]
	createdAt?: string
	cluster_url?: string
	cluster_name?: string
	raw?: ListPipelineResponseInner | PipelineDetail
}

export interface IHostSelectorItem {
	id: string
	name: string
	ip: string
	environment: 'prod' | 'staging' | 'dev'
}

export interface ISinkSelectorItem {
	id: string
	name: string
	type: string
}

export interface IPipelineCreateForm {
	name: string
	cluster_url: string
	access: PipelineAccess
	visibility: number
	adm_status: number
	accessibleUsers: string[]
	assignedHosts: string[]
	assignedSinks: string[]
	last_updated?: string
}

export type {
	CreatePipelineRequest,
	ListPipelineResponseInner,
	PipelineDetail,
	UpdatePipelineRequest
}

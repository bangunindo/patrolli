// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { ActivityState } from '@/app/helpers/itemstats.helper'
import {
	CreateHostItemBody,
	CreateTemplateItemBody,
	GetItemResponse,
	ListItemsQueryParams,
	ListItemsResponse
} from '@@/shared/generated/items.type'
import { z } from 'zod'
import type { IBaseResponse, IFilterParam } from '@@/shared/types/api.type'

export const ZodItem = GetItemResponse.shape.data.unwrap()
export const ZodItems = ListItemsResponse.shape.data.unwrap().shape.items.element
const ZodItemConfig = CreateHostItemBody.shape.config

export type ItemType = z.infer<typeof ZodItemConfig>['type']

export const ItemType = ZodItemConfig.options.reduce(
	(acc, option) => {
		const typeValue = option.shape.type.options[0] as ItemType

		const uppercaseKey = typeValue.toUpperCase() as Uppercase<ItemType>

		acc[uppercaseKey] = typeValue

		return acc
	},
	{} as Record<Uppercase<ItemType>, ItemType>
)

export interface IItems extends z.infer<typeof ZodItems> {
	cluster_url?: string
	cluster_name?: string
}

export interface IItem extends z.infer<typeof ZodItem> {
	cluster_url?: string
	cluster_name?: string
}

export enum ItemCategory {
	HOST_ONLY = 'HOST_ONLY',
	APPLIED_TEMPLATE = 'APPLIED_TEMPLATE',
	TEMPLATE_ROOT = 'TEMPLATE_ROOT',
	TEMPLATE_INHERITED = 'TEMPLATE_INHERITED'
}

const preprocessNumber = (val: any) => {
	if (val === '' || val === null || val === undefined || Number.isNaN(val)) return undefined
	return Number(val)
}

export const ItemFormSchema = CreateHostItemBody.extend({
	name: z.string().min(1, 'Name is required'),
	host_id: z.string().min(1, 'Host is required'),
	schedule: z.string().min(1, 'Schedule is required'),
	adm_status: z.number({ required_error: 'Status is required' }),
	flags: z.preprocess(
		preprocessNumber,
		z.number({
			required_error: 'Flags is required',
			invalid_type_error: 'Flags must be a valid number'
		})
	),
	timeout_ms: z.preprocess(
		preprocessNumber,
		z
			.number({
				required_error: 'Timeout is required',
				invalid_type_error: 'Timeout must be a valid number'
			})
			.min(1, 'Timeout must be at least 1')
	)
})

export const TemplateItemFormSchema = CreateTemplateItemBody.extend({
	name: z.string().min(1, 'Name is required'),
	template_id: z.string().min(1, 'Template is required'),
	schedule: z.string().min(1, 'Schedule is required'),
	adm_status: z.number({ required_error: 'Status is required' }),
	flags: z.preprocess(
		preprocessNumber,
		z.number({
			required_error: 'Flags is required',
			invalid_type_error: 'Flags must be a valid number'
		})
	),
	timeout_ms: z.preprocess(
		preprocessNumber,
		z
			.number({
				required_error: 'Timeout is required',
				invalid_type_error: 'Timeout must be a valid number'
			})
			.min(1, 'Timeout must be at least 1')
	)
})

export interface IItemForm extends z.infer<typeof ItemFormSchema> {}
export interface IItemTemplateForm extends z.infer<typeof CreateTemplateItemBody> {}

export interface IItemListParams
	extends Partial<z.infer<typeof ListItemsQueryParams>>, IFilterParam {
	name?: string
	order_by?: 'item_id' | 'name' | 'last_updated'
	host_template_ids?: string[]
	host_ids?: string[]
	template_ids?: string[]
	types?: ItemType[]
	endpoint_ids?: string[]
	listener_ids?: string[]
	processor_id?: string
	exclusivity?: any[]
}

export interface IItemListResponse extends IBaseResponse {
	data: {
		items: IItems[]
		total: number
	}
}

export interface IItemResponse extends IBaseResponse {
	data?: IItem
}

export interface IItemSelect extends IItem {
	label: string
	value: string
}

export interface IStateMeta {
	label: string
	sublabel: string
	color: string
	dotClass: string
	icon: string
}

export type StateMeta = IStateMeta

export interface IFailureKindMeta {
	label: string
	category: 'Target' | 'Config' | 'Patrolli'
	hint: string
	color: string
}

export type FailureKindMeta = IFailureKindMeta

export interface IActivityBucket {
	step: number
	time: Date
	startTime: string
	endTime: string
	timeLabel: string
	state: ActivityState
}

export type ActivityBucket = IActivityBucket

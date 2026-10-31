// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type {
	CreateProcessorBody,
	ListProcessorsQueryParams
} from '@@/shared/generated/processors.type'
import {
	AttributeFilter,
	ListProcessorResponseInner,
	type AddAttributes,
	type DropAttributes,
	type RenameAttributes
} from '@@/shared/generated/patrolliAPI.schemas.type'
import type z from 'zod'
import type { IBaseResponse, IFilterParam } from '@@/shared/types/api.type'
import type { ISelectOption } from '@@/shared/types/app.type'

export type AttributeLookupCast = 'bool' | 'integer' | 'string' | 'ip'

export interface IAttributeLookupInput {
	key: string
	cast?: AttributeLookupCast | null
	table?: 'record' | 'scope' | 'resource' | null
}

export interface IAttributeLookupMatch {
	keys?: Record<string, IAttributeLookupInput>
	point?: IAttributeLookupInput | null
}

export interface IAttributeLookupRule {
	lookup_table: string
	table: 'record' | 'scope' | 'resource'
	match: IAttributeLookupMatch
	output: Record<string, string>
	when?: AttributeFilter | null
}

export interface IAttributeLookup {
	lookups: IAttributeLookupRule[]
	overwrite?: boolean
	when?: AttributeFilter | null
}

export type ProcessorConfigMap = {
	null: undefined
	attribute_filter: AttributeFilter
	drop_attributes: DropAttributes
	add_attributes: AddAttributes
	rename_attributes: RenameAttributes
	attribute_lookup: IAttributeLookup
}

export interface IProcessorConfig<T extends keyof ProcessorConfigMap = keyof ProcessorConfigMap> {
	type?: T
	module?: T
	config: ProcessorConfigMap[T]
}

export interface IProcessor<
	T extends keyof ProcessorConfigMap = keyof ProcessorConfigMap
> extends Omit<z.infer<typeof ListProcessorResponseInner>, 'adm_status' | 'config'> {
	config: { type?: T; module?: T; config?: ProcessorConfigMap[T] | any }
	adm_status: number
	cluster_url?: string
	cluster_name?: string
}

export interface IProcessorForm<
	T extends keyof ProcessorConfigMap = keyof ProcessorConfigMap
> extends Omit<z.infer<typeof CreateProcessorBody>, 'config'> {
	config: { type?: T; module?: T; config?: ProcessorConfigMap[T] | any }
}

export interface IProcessorListParams
	extends
		Omit<Partial<z.infer<typeof ListProcessorsQueryParams>>, keyof IFilterParam>,
		IFilterParam {}

export interface IProcessorListResponse extends IBaseResponse {
	data: {
		processors: IProcessor[]
		total: number
	}
}

export interface IProcessorResponse extends IBaseResponse {
	data?: IProcessor
}

export interface IProcessorSelect extends IProcessor, ISelectOption {}

// Processor Config Module Type
export type ProcessorType = keyof ProcessorConfigMap

export const ProcessorType: Record<string, ProcessorType> = {
	NULL: 'null',
	ATTRIBUTE_FILTER: 'attribute_filter',
	DROP_ATTRIBUTES: 'drop_attributes',
	ADD_ATTRIBUTES: 'add_attributes',
	RENAME_ATTRIBUTES: 'rename_attributes',
	ATTRIBUTE_LOOKUP: 'attribute_lookup'
}

export const ProcessorTypeLabels: Record<ProcessorType, string> = {
	null: 'Null / Passthrough',
	attribute_filter: 'Attribute Filter',
	drop_attributes: 'Drop Attributes',
	add_attributes: 'Add Attributes',
	rename_attributes: 'Rename Attributes',
	attribute_lookup: 'Attribute Lookup (Data Enrichment)'
}

// Attribute Filter
type KeysOfUnion<T> = T extends any ? keyof T : never
const attributeFilter = AttributeFilter as any
export type ProcessorAttributeFilter = KeysOfUnion<AttributeFilter>
export const ProcessorAttributeFilter = attributeFilter.options.flatMap((option: any) => {
	return Object.keys(option.shape)
})

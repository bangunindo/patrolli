// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

/** This is temporary File, it will be removed in the future */

export interface IConfig {
	id: string
	name: string
	visible_name: string
	description: string
}

export interface IHostDummy {
	id: string
	name: string
	visible_name: string
	description: string
	use_proxy?: boolean
	groups?: IHostGroup['id'][]
	endpoints?: IHostEndpoint['id'][]
	templates?: IHostTemplate['id'][]
}

export interface IHostGroup {
	id: string
	name: string
}

export interface IHostEndpoint {
	id: string
	name: string
}

export interface IHostTemplate {
	id: string
	name: string
	items: IHostTemplateItem['id'][]
}

export interface IHostTemplateItem {
	id: string
	name: string
	type: IHostTemplateItemType['id']
}

export interface IHostTemplateItemType {
	id: string
	name: string
	fields: IHostTemplateItemTypeField['id'][]
}

export interface IHostTemplateItemTypeField {
	id: string
	name: string
	type: TItemTypeFieldType
	inputType?: TItemTypeFieldInputType
	optionType?: TItemTypeFieldOptionType
	optionList?: {
		key: string
		label: string
	}[]
	optionMultiple?: boolean
	defaultValue: string
}

export type TItemTypeFieldType = 'input' | 'option'

export type TItemTypeFieldInputType = 'text' | 'number'

export type TItemTypeFieldOptionType = 'radio' | 'checkbox' | 'select'

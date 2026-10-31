// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

export type TOrderType = 'asc' | 'desc'

export type TAdmStatus = 0 | 1

export type TAdmStatusString = 'Disabled' | 'Enabled'

export interface IBaseResponse {
	success: boolean
	status_code: number
	data?: any
	total_data?: number
	error?: {
		detail: string
		title: string
	}
}

export interface IFilterParam {
	limit?: number
	offset?: number
	order_by?: string
	order_dir?: TOrderType
}

export interface IPaginationData {
	total: number
	per_page: number
	current_page: number
	total_pages: number
	offset: number
}

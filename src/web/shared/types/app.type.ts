// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

export declare interface IApiResponse {
	status: string
	message: string
}

export declare interface IUser {
	id: number
	username: string
	fullname: string
	email: string
	email_verified_at: any
	created_at: string
	updated_at: string
}

export type ToastOptions = {
	description: string
	title?: string | null
	duration?: number
}

export enum FormMode {
	ADD = 'ADD',
	UPDATE = 'UPDATE'
}

export interface IPaginationTable {
	totalRecords: number
	rows: number
	currentPage: number
}

export interface ISelectOption {
	label: string
	value: any
}

export type AppEnum<T extends string> = Record<Uppercase<T>, T>

// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

export interface IClusterClientItem {
	key: string
	name: string
	url: string
	clientPath: string
	label: string
	value: string
}

export interface IClustersResponse {
	status_code: number
	success: boolean
	data: IClusterClientItem[]
}

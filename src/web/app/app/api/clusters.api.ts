// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { IClusterListResponse } from '@@/shared/types/cluster.type'
import { httpService } from '@/app/services/http.service'

export interface IFormattedClusterOption {
	alias: string
	name: string
	label: string
	value: string
}

export function formatClusterOptions(rawList: unknown): IFormattedClusterOption[] {
	if (!Array.isArray(rawList)) return []
	return rawList.map((item: any) => {
		if (typeof item === 'string') {
			return {
				alias: item,
				name: item,
				label: item,
				value: item
			}
		}
		const name = item.name || item.key || item.value
		return {
			alias: item.key || item.alias || name,
			name,
			label: item.label || name,
			value: item.value || item.clientPath || item.url || name
		}
	})
}

const clustersApi = {
	path: '/api/clusters',

	async getClusters(params?: { refresh?: boolean }): Promise<IClusterListResponse | undefined> {
		return httpService.get<IClusterListResponse>(this.path, {
			params
		})
	}
}

export default clustersApi

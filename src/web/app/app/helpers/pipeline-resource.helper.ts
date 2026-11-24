// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import hostsApi from '@/app/api/hosts.api'
import sinksApi from '@/app/api/sinks.api'
import {
	createPaginatedResource,
	type PaginatedResourceConfig
} from '@/app/helpers/pagination.helper'

export interface PipelineResourceFactoryOptions<TItem> extends Omit<
	PaginatedResourceConfig<any, TItem>,
	'fetchPage' | 'fetchByIds'
> {
	defaultErrorMessage?: string
}

function createApiPaginatedResource<TItem>(
	options: PipelineResourceFactoryOptions<TItem>,
	adapter: {
		fetch: (query: Record<string, any>, cUrls?: string[]) => Promise<any>
		listKey: 'hosts' | 'sinks'
		idsParamKey: 'host_ids' | 'sink_ids'
		fallbackErrorMessage: string
	}
) {
	const { defaultErrorMessage = adapter.fallbackErrorMessage, ...rest } = options
	return createPaginatedResource<any, TItem>({
		...rest,
		fetchPage: async (query, cUrls) => {
			const res = await adapter.fetch(query, cUrls)
			if (!res || res.success === false) {
				throw new Error((res as any)?.message || defaultErrorMessage)
			}
			return {
				items: res.data?.[adapter.listKey] || [],
				total: res.data?.total
			}
		},
		fetchByIds: async (ids, cUrls) => {
			const res = await adapter.fetch(
				{ [adapter.idsParamKey]: ids, limit: ids.length },
				cUrls
			)
			return res?.success !== false ? res?.data?.[adapter.listKey] || [] : []
		}
	})
}

export function createHostsPaginatedResource<TItem = any>(
	options: PipelineResourceFactoryOptions<TItem>
) {
	return createApiPaginatedResource(options, {
		fetch: (query, cUrls) => hostsApi.getHosts(query, cUrls),
		listKey: 'hosts',
		idsParamKey: 'host_ids',
		fallbackErrorMessage: 'Failed to load hosts.'
	})
}

export function createSinksPaginatedResource<TItem = any>(
	options: PipelineResourceFactoryOptions<TItem>
) {
	return createApiPaginatedResource(options, {
		fetch: (query, cUrls) => sinksApi.getSinks(query, cUrls),
		listKey: 'sinks',
		idsParamKey: 'sink_ids',
		fallbackErrorMessage: 'Failed to load sinks.'
	})
}

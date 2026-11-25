// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { ref, type Ref } from 'vue'

export interface FetchAllPagesOptions {
	pageSize?: number
	concurrency?: number
}

const SELECTOR_PAGE_SIZE = 100
export const DEFAULT_PAGE_SIZE = 500
export const DEFAULT_CONCURRENCY = 5

export async function fetchAllPages<T>(
	fetchPage: (limit: number, offset: number) => Promise<{ items: T[]; total?: number }>,
	options: FetchAllPagesOptions = {}
): Promise<T[]> {
	const pageSize = Math.max(1, options.pageSize ?? DEFAULT_PAGE_SIZE)
	const concurrency = Math.max(1, options.concurrency ?? DEFAULT_CONCURRENCY)

	const firstPage = await fetchPage(pageSize, 0)
	const collected: T[] = [...firstPage.items]

	if (
		firstPage.items.length === 0 ||
		(typeof firstPage.total === 'number'
			? collected.length >= firstPage.total
			: firstPage.items.length < pageSize)
	) {
		return collected
	}

	if (typeof firstPage.total === 'number' && firstPage.total > collected.length) {
		const effectivePageSize = Math.min(pageSize, firstPage.items.length)
		const totalPages = Math.ceil(firstPage.total / effectivePageSize)
		for (let startPage = 1; startPage < totalPages; startPage += concurrency) {
			const endPage = Math.min(totalPages, startPage + concurrency)
			const batchPromises: Promise<{ items: T[]; total?: number }>[] = []
			for (let page = startPage; page < endPage; page++) {
				batchPromises.push(fetchPage(effectivePageSize, page * effectivePageSize))
			}
			const batchResults = await Promise.all(batchPromises)
			let hitEmptyPage = false
			for (const pageRes of batchResults) {
				collected.push(...pageRes.items)
				if (pageRes.items.length === 0) {
					hitEmptyPage = true
				}
			}
			if (hitEmptyPage || collected.length >= firstPage.total) {
				break
			}
		}
		return collected
	}

	for (let page = 1; ; page++) {
		const { items, total } = await fetchPage(pageSize, page * pageSize)
		collected.push(...items)
		if (
			items.length === 0 ||
			items.length < pageSize ||
			(typeof total === 'number' && collected.length >= total)
		) {
			break
		}
	}

	return collected
}

export interface HydrateByIdsOptions {
	chunkSize?: number
	maxIds?: number
}

export interface PaginatedResourceConfig<TRaw, TItem> {
	fetchPage: (
		query: Record<string, any>,
		clusterURLs?: string[]
	) => Promise<{ items: TRaw[]; total?: number }>
	fetchByIds?: (ids: string[], clusterURLs?: string[]) => Promise<TRaw[]>
	mapItem: (raw: TRaw) => TItem
	getId: (item: TItem) => string
	pageSize?: number
	clearOnError?: boolean
	onError?: (err: unknown, stage: 'first' | 'more') => void
	onLoadingChange?: (loading: boolean, loadingMore: boolean) => void
}

export function createPaginatedResource<TRaw, TItem = TRaw>(
	config: PaginatedResourceConfig<TRaw, TItem>
) {
	const pageSize = config.pageSize ?? SELECTOR_PAGE_SIZE
	const items = ref<TItem[]>([]) as Ref<TItem[]>
	const total = ref(0)
	const searchQuery = ref('')
	const loading = ref(false)
	const loadingMore = ref(false)
	const cache = ref<Record<string, TItem>>({}) as Ref<Record<string, TItem>>

	let requestSeq = 0

	function notifyLoading() {
		config.onLoadingChange?.(loading.value, loadingMore.value)
	}

	function mergeCache(list: TItem[]) {
		if (list.length === 0) return
		const next = { ...cache.value }
		for (const item of list) {
			const id = config.getId(item)
			if (id) next[id] = item
		}
		cache.value = next
	}

	function buildQuery(limit: number, offset: number, search: string): Record<string, any> {
		const query: Record<string, any> = offset > 0 ? { limit, offset } : { limit }
		if (search) query.name = search
		return query
	}

	async function fetchFirst(clusterURLs?: string[], search = '') {
		const seq = ++requestSeq
		const trimmed = search.trim()
		searchQuery.value = trimmed
		loading.value = true
		loadingMore.value = false
		notifyLoading()
		try {
			const res = await config.fetchPage(buildQuery(pageSize, 0, trimmed), clusterURLs)
			if (seq !== requestSeq) return
			const mapped = res.items.map(config.mapItem)
			items.value = mapped
			total.value = typeof res.total === 'number' ? res.total : mapped.length
			mergeCache(mapped)
		} catch (err) {
			if (seq === requestSeq) {
				if (config.clearOnError) {
					items.value = []
					total.value = 0
				}
				config.onError?.(err, 'first')
			}
		} finally {
			if (seq === requestSeq) {
				loading.value = false
				notifyLoading()
			}
		}
	}

	async function loadMore(clusterURLs?: string[]) {
		if (loading.value || loadingMore.value || items.value.length >= total.value) {
			return
		}
		const seq = requestSeq
		loadingMore.value = true
		notifyLoading()
		try {
			const res = await config.fetchPage(
				buildQuery(pageSize, items.value.length, searchQuery.value),
				clusterURLs
			)
			if (seq !== requestSeq) return
			const mapped = res.items.map(config.mapItem)
			const existingIds = new Set(items.value.map(config.getId))
			const nextItems = mapped.filter((item) => !existingIds.has(config.getId(item)))
			if (nextItems.length > 0) {
				items.value = [...items.value, ...nextItems]
				mergeCache(nextItems)
			}
			if (
				res.items.length === 0 ||
				nextItems.length === 0 ||
				(typeof res.total !== 'number' && res.items.length < pageSize)
			) {
				total.value = items.value.length
			} else if (typeof res.total === 'number') {
				total.value = res.total
			}
		} catch (err) {
			if (seq === requestSeq) {
				config.onError?.(err, 'more')
			}
		} finally {
			if (seq === requestSeq) {
				loadingMore.value = false
				notifyLoading()
			}
		}
	}

	async function resolveAllIds(clusterURLs?: string[], search = ''): Promise<string[]> {
		const seq = requestSeq
		const trimmed = search.trim()
		const rawItems = await fetchAllPages(async (limit, offset) => {
			return config.fetchPage(buildQuery(limit, offset, trimmed), clusterURLs)
		})
		const mapped = rawItems.map(config.mapItem)
		if (seq === requestSeq) {
			mergeCache(mapped)
		}
		return Array.from(new Set(mapped.map(config.getId).filter(Boolean)))
	}

	async function hydrateByIds(
		ids: string[],
		clusterURLs?: string[],
		options: HydrateByIdsOptions = {}
	): Promise<void> {
		if (!config.fetchByIds || ids.length === 0) return

		const uniqueMissing: string[] = []
		const seen = new Set<string>()
		const limit =
			typeof options.maxIds === 'number'
				? Math.max(0, options.maxIds)
				: Number.POSITIVE_INFINITY

		for (const rawId of ids) {
			if (!rawId || seen.has(rawId)) continue
			seen.add(rawId)
			if (cache.value[rawId]) continue
			uniqueMissing.push(rawId)
			if (uniqueMissing.length >= limit) break
		}

		if (uniqueMissing.length === 0) return

		const seq = requestSeq
		const chunkSize = Math.max(1, options.chunkSize ?? 50)
		const chunks: string[][] = []
		for (let i = 0; i < uniqueMissing.length; i += chunkSize) {
			chunks.push(uniqueMissing.slice(i, i + chunkSize))
		}

		for (let start = 0; start < chunks.length; start += DEFAULT_CONCURRENCY) {
			if (seq !== requestSeq) return
			const batch = chunks.slice(start, start + DEFAULT_CONCURRENCY)
			const results = await Promise.allSettled(
				batch.map((chunk) => config.fetchByIds!(chunk, clusterURLs))
			)
			if (seq !== requestSeq) return
			const fetched: TItem[] = []
			for (const res of results) {
				if (res.status === 'fulfilled' && Array.isArray(res.value)) {
					fetched.push(...res.value.map(config.mapItem))
				}
			}
			mergeCache(fetched)
		}
	}

	function reset() {
		requestSeq++
		items.value = []
		total.value = 0
		searchQuery.value = ''
		cache.value = {}
		loading.value = false
		loadingMore.value = false
		notifyLoading()
	}

	return {
		items,
		total,
		searchQuery,
		loading,
		loadingMore,
		cache,
		fetchFirst,
		loadMore,
		resolveAllIds,
		hydrateByIds,
		reset
	}
}

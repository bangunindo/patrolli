// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it, vi } from 'vitest'
import {
	createPaginatedResource,
	DEFAULT_CONCURRENCY,
	DEFAULT_PAGE_SIZE,
	fetchAllPages
} from '@/app/helpers/pagination.helper'

describe('fetchAllPages', () => {
	it('returns immediately on a single page response', async () => {
		const fetchPage = vi.fn().mockResolvedValueOnce({
			items: [{ id: 1 }, { id: 2 }],
			total: 2
		})

		const result = await fetchAllPages(fetchPage)

		expect(fetchPage).toHaveBeenCalledTimes(1)
		expect(fetchPage).toHaveBeenCalledWith(DEFAULT_PAGE_SIZE, 0)
		expect(result).toEqual([{ id: 1 }, { id: 2 }])
	})

	it('fetches remaining pages in concurrency-bounded batches when total is known', async () => {
		let active = 0
		let maxActive = 0
		const total = 12 * DEFAULT_PAGE_SIZE + 10

		const fetchPage = vi.fn(async (limit: number, offset: number) => {
			active++
			maxActive = Math.max(maxActive, active)
			await Promise.resolve()
			const remaining = Math.max(0, total - offset)
			const count = Math.min(limit, remaining)
			active--
			return {
				items: Array.from({ length: count }, (_, idx) => offset + idx),
				total
			}
		})

		const result = await fetchAllPages(fetchPage, {
			pageSize: DEFAULT_PAGE_SIZE,
			concurrency: DEFAULT_CONCURRENCY
		})

		expect(result).toHaveLength(total)
		expect(fetchPage).toHaveBeenCalledTimes(13)
		expect(maxActive).toBeLessThanOrEqual(DEFAULT_CONCURRENCY)
	})

	it('paginates across many pages when total is known', async () => {
		const pageSize = 10
		const total = 605 // 61 pages
		const fetchPage = vi.fn(async (limit: number, offset: number) => {
			const remaining = Math.max(0, total - offset)
			const count = Math.min(limit, remaining)
			return {
				items: Array.from({ length: count }, (_, idx) => `id-${offset + idx}`),
				total
			}
		})

		const result = await fetchAllPages(fetchPage, { pageSize, concurrency: 5 })

		expect(fetchPage).toHaveBeenCalledTimes(61)
		expect(result).toHaveLength(605)
		expect(result[604]).toBe('id-604')
	})

	it('paginates across many pages when total is unknown', async () => {
		const pageSize = 10
		const total = 553 // 56 pages
		const fetchPage = vi.fn(async (limit: number, offset: number) => {
			const remaining = Math.max(0, total - offset)
			const count = Math.min(limit, remaining)
			return {
				items: Array.from({ length: count }, (_, idx) => `id-${offset + idx}`)
			}
		})

		const result = await fetchAllPages(fetchPage, { pageSize })

		expect(fetchPage).toHaveBeenCalledTimes(56)
		expect(result).toHaveLength(553)
	})

	it('stops early if server returns empty page before total is reached', async () => {
		const fetchPage = vi
			.fn()
			.mockResolvedValueOnce({ items: ['a', 'b'], total: 10 })
			.mockResolvedValueOnce({ items: [], total: 10 })
			.mockResolvedValueOnce({ items: [], total: 10 })

		const result = await fetchAllPages(fetchPage, { pageSize: 2, concurrency: 2 })

		expect(result).toEqual(['a', 'b'])
		expect(fetchPage).toHaveBeenCalledTimes(3)
	})

	it('continues paginating with effectivePageSize when server clamps limit below requested pageSize', async () => {
		const total = 350
		const serverMaxLimit = 100
		const fetchPage = vi.fn(async (limit: number, offset: number) => {
			const effective = Math.min(limit, serverMaxLimit)
			const remaining = Math.max(0, total - offset)
			const count = Math.min(effective, remaining)
			return {
				items: Array.from({ length: count }, (_, idx) => `item-${offset + idx}`),
				total
			}
		})

		const result = await fetchAllPages(fetchPage, { pageSize: 500, concurrency: 2 })

		expect(result).toHaveLength(total)
		expect(result[0]).toBe('item-0')
		expect(result[349]).toBe('item-349')
		expect(fetchPage).toHaveBeenNthCalledWith(1, 500, 0)
		expect(fetchPage).toHaveBeenNthCalledWith(2, 100, 100)
		expect(fetchPage).toHaveBeenNthCalledWith(3, 100, 200)
		expect(fetchPage).toHaveBeenNthCalledWith(4, 100, 300)
		expect(fetchPage).toHaveBeenCalledTimes(4)
	})
})

describe('createPaginatedResource', () => {
	it('clamps total to loaded items length when loadMore returns empty or duplicate-only page', async () => {
		const fetchPage = vi
			.fn()
			.mockResolvedValueOnce({
				items: [
					{ id: '1', name: 'One' },
					{ id: '2', name: 'Two' }
				],
				total: 10
			})
			.mockResolvedValueOnce({
				items: [{ id: '2', name: 'Two' }],
				total: 10
			})

		const resource = createPaginatedResource({
			fetchPage,
			mapItem: (raw: { id: string; name: string }) => raw,
			getId: (item) => item.id,
			pageSize: 2
		})

		await resource.fetchFirst()
		expect(resource.items.value).toHaveLength(2)
		expect(resource.total.value).toBe(10)

		await resource.loadMore()
		expect(resource.items.value).toHaveLength(2)
		expect(resource.total.value).toBe(2)

		// Subsequent loadMore should be a no-op since total is clamped
		await resource.loadMore()
		expect(fetchPage).toHaveBeenCalledTimes(2)
	})

	it('deduplicates IDs in resolveAllIds across pages', async () => {
		const fetchPage = vi
			.fn()
			.mockResolvedValueOnce({
				items: Array.from({ length: DEFAULT_PAGE_SIZE }, (_, idx) => ({ id: `id-${idx}` })),
				total: DEFAULT_PAGE_SIZE + 2
			})
			.mockResolvedValueOnce({
				items: [{ id: `id-${DEFAULT_PAGE_SIZE - 1}` }, { id: 'id-extra' }],
				total: DEFAULT_PAGE_SIZE + 2
			})

		const resource = createPaginatedResource({
			fetchPage,
			mapItem: (raw: { id: string }) => raw,
			getId: (item) => item.id
		})

		const ids = await resource.resolveAllIds()
		expect(ids).toHaveLength(DEFAULT_PAGE_SIZE + 1)
		expect(new Set(ids).size).toBe(ids.length)
	})

	it('clears state and ignores in-flight fetchFirst after reset', async () => {
		let resolveFirst!: (val: { items: { id: string }[]; total: number }) => void
		const fetchPage = vi.fn(
			() =>
				new Promise<{ items: { id: string }[]; total: number }>((resolve) => {
					resolveFirst = resolve
				})
		)

		const resource = createPaginatedResource({
			fetchPage,
			mapItem: (raw: { id: string }) => raw,
			getId: (item) => item.id
		})

		const pending = resource.fetchFirst(undefined, 'alpha')
		expect(resource.searchQuery.value).toBe('alpha')
		expect(resource.loading.value).toBe(true)

		resource.reset()
		expect(resource.items.value).toEqual([])
		expect(resource.total.value).toBe(0)
		expect(resource.searchQuery.value).toBe('')
		expect(resource.cache.value).toEqual({})
		expect(resource.loading.value).toBe(false)

		resolveFirst({ items: [{ id: 'stale-1' }], total: 1 })
		await pending
		expect(resource.items.value).toEqual([])
		expect(resource.cache.value).toEqual({})
	})

	it('does not pollute cache when reset is called while resolveAllIds is in flight', async () => {
		let resolveFetch!: (val: { items: { id: string; name: string }[]; total: number }) => void
		const fetchPage = vi.fn(
			() =>
				new Promise<{ items: { id: string; name: string }[]; total: number }>((resolve) => {
					resolveFetch = resolve
				})
		)

		const resource = createPaginatedResource({
			fetchPage,
			mapItem: (raw: { id: string; name: string }) => raw,
			getId: (item) => item.id
		})

		const pending = resource.resolveAllIds()
		resource.reset()

		resolveFetch({ items: [{ id: 'old-1', name: 'Old Cluster Host' }], total: 1 })
		const ids = await pending

		expect(ids).toEqual(['old-1'])
		expect(resource.cache.value).toEqual({})
	})

	it('continues loadMore when server clamps limit below pageSize but reports total', async () => {
		const fetchPage = vi
			.fn()
			.mockResolvedValueOnce({
				items: [
					{ id: '1', name: 'One' },
					{ id: '2', name: 'Two' }
				],
				total: 5
			})
			.mockResolvedValueOnce({
				items: [
					{ id: '3', name: 'Three' },
					{ id: '4', name: 'Four' }
				],
				total: 5
			})
			.mockResolvedValueOnce({
				items: [{ id: '5', name: 'Five' }],
				total: 5
			})

		const resource = createPaginatedResource({
			fetchPage,
			mapItem: (raw: { id: string; name: string }) => raw,
			getId: (item) => item.id,
			pageSize: 10
		})

		await resource.fetchFirst()
		expect(resource.items.value).toHaveLength(2)
		expect(resource.total.value).toBe(5)

		await resource.loadMore()
		expect(resource.items.value).toHaveLength(4)
		expect(resource.total.value).toBe(5)

		await resource.loadMore()
		expect(resource.items.value).toHaveLength(5)
		expect(resource.total.value).toBe(5)
		expect(fetchPage).toHaveBeenCalledTimes(3)
	})

	it('hydrates missing IDs in chunks, skips cached IDs, respects maxIds, and ignores stale results after reset', async () => {
		const fetchByIds = vi.fn(async (ids: string[]) =>
			ids.map((id) => ({ id, name: `Name ${id}` }))
		)
		const resource = createPaginatedResource({
			fetchPage: vi.fn().mockResolvedValue({
				items: [{ id: 'id-0', name: 'Cached 0' }],
				total: 10
			}),
			fetchByIds,
			mapItem: (raw: { id: string; name: string }) => raw,
			getId: (item) => item.id
		})

		await resource.fetchFirst()
		await resource.hydrateByIds(
			['id-0', 'id-1', 'id-2', 'id-3', 'id-2', 'id-4'],
			['http://c1.test:8080'],
			{ chunkSize: 2, maxIds: 3 }
		)

		expect(fetchByIds).toHaveBeenCalledTimes(2)
		expect(fetchByIds).toHaveBeenNthCalledWith(1, ['id-1', 'id-2'], ['http://c1.test:8080'])
		expect(fetchByIds).toHaveBeenNthCalledWith(2, ['id-3'], ['http://c1.test:8080'])
		expect(resource.cache.value['id-0']?.name).toBe('Cached 0')
		expect(resource.cache.value['id-1']?.name).toBe('Name id-1')
		expect(resource.cache.value['id-3']?.name).toBe('Name id-3')
		expect(resource.cache.value['id-4']).toBeUndefined()

		let resolveHydrate!: (val: { id: string; name: string }[]) => void
		fetchByIds.mockReturnValueOnce(
			new Promise((resolve) => {
				resolveHydrate = resolve
			})
		)
		const pending = resource.hydrateByIds(['id-stale'])
		resource.reset()
		resolveHydrate([{ id: 'id-stale', name: 'Stale' }])
		await pending
		expect(resource.cache.value).toEqual({})
	})
})

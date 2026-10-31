// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { distributePagination, rebalancePaginationResponses } from '@@/server/utils/pagination'
import * as clusterUtils from '@@/server/services/cluster.service'

import type { ICluster } from '@@/shared/types/cluster.type'

describe('Pagination Utilities & Rebalancing', () => {
	beforeEach(() => {
		vi.restoreAllMocks()
	})

	describe('distributePagination', () => {
		it('should return exact global offset and limit when nodeCount <= 1', () => {
			const result = distributePagination(10, 20, 1, 0)
			expect(result).toEqual({ offset: 10, limit: 20 })
		})

		it('should distribute limit and offset evenly between 2 nodes', () => {
			// Limit 10, Offset 0 -> node 0 gets 5, node 1 gets 5
			expect(distributePagination(0, 10, 2, 0)).toEqual({ offset: 0, limit: 5 })
			expect(distributePagination(0, 10, 2, 1)).toEqual({ offset: 0, limit: 5 })

			// Limit 10, Offset 10 (page 2) -> node 0 gets 5 (offset 5), node 1 gets 5 (offset 5)
			expect(distributePagination(10, 10, 2, 0)).toEqual({ offset: 5, limit: 5 })
			expect(distributePagination(10, 10, 2, 1)).toEqual({ offset: 5, limit: 5 })
		})

		it('should handle odd limits across nodes', () => {
			// Limit 5 across 2 nodes -> node 0 gets 3, node 1 gets 2
			expect(distributePagination(0, 5, 2, 0)).toEqual({ offset: 0, limit: 3 })
			expect(distributePagination(0, 5, 2, 1)).toEqual({ offset: 0, limit: 2 })
		})
	})

	describe('rebalancePaginationResponses', () => {
		it('should fetch missing items from second cluster when first cluster has fewer items', async () => {
			const cluster1: ICluster = {
				key: 'node1',
				name: 'node1',
				url: 'http://localhost:8081',
				clientPath: '/node1'
			}
			const cluster2: ICluster = {
				key: 'node2',
				name: 'node2',
				url: 'http://localhost:8082',
				clientPath: '/node2'
			}

			// Merged result initially has 1 item from node1 and 5 items from node2 (total 6 items, but requested 10 items)
			// Total global items available: 50 (node1 has 1, node2 has 49)
			const mergedResult = {
				status_code: 200,
				success: true,
				data: {
					items: [
						{ id: '1-1', cluster_name: 'node1', cluster_url: '/node1' },
						{ id: '2-1', cluster_name: 'node2', cluster_url: '/node2' },
						{ id: '2-2', cluster_name: 'node2', cluster_url: '/node2' },
						{ id: '2-3', cluster_name: 'node2', cluster_url: '/node2' },
						{ id: '2-4', cluster_name: 'node2', cluster_url: '/node2' },
						{ id: '2-5', cluster_name: 'node2', cluster_url: '/node2' }
					],
					total: 50
				}
			}

			const initialResponses = [
				{
					cluster: cluster1,
					plan: { localOffset: 0, localLimit: 5 },
					data: { data: { items: [{ id: '1-1' }], total: 1 } },
					status: 200
				},
				{
					cluster: cluster2,
					plan: { localOffset: 0, localLimit: 5 },
					data: {
						data: {
							items: [
								{ id: '2-1' },
								{ id: '2-2' },
								{ id: '2-3' },
								{ id: '2-4' },
								{ id: '2-5' }
							],
							total: 49
						}
					},
					status: 200
				}
			]

			// Mock forwardToCluster for extra fetch from cluster2 (offset: 5, limit: 4)
			const forwardSpy = vi.spyOn(clusterUtils, 'forwardToCluster').mockResolvedValue({
				status: 200,
				headers: new Headers(),
				data: {
					data: {
						items: [{ id: '2-6' }, { id: '2-7' }, { id: '2-8' }, { id: '2-9' }],
						total: 49
					}
				}
			})

			await rebalancePaginationResponses({
				mergedResult,
				initialResponses,
				fullApiPath: '/api/items',
				method: 'GET',
				reqHeaders: {},
				query: {},
				offset: 0,
				limit: 10
			})

			// Forward should be called for cluster2 with offset: 5 and limit: 4
			expect(forwardSpy).toHaveBeenCalledWith(
				cluster2,
				'/api/items',
				expect.objectContaining({
					query: expect.objectContaining({ offset: 5, limit: 4 })
				})
			)

			// Total items in mergedResult should now be 10 (6 initial + 4 extra)
			expect(mergedResult.data.items.length).toBe(10)
			expect(mergedResult.data.items[6]).toMatchObject({
				id: '2-6',
				cluster_name: 'node2',
				cluster_url: '/node2'
			})
		})

		it('should do nothing if fetched items already satisfy limit', async () => {
			const forwardSpy = vi.spyOn(clusterUtils, 'forwardToCluster')

			const mergedResult = {
				data: {
					items: new Array(10).fill({ id: 'x' }),
					total: 20
				}
			}

			await rebalancePaginationResponses({
				mergedResult,
				initialResponses: [],
				fullApiPath: '/api/items',
				method: 'GET',
				reqHeaders: {},
				query: {},
				offset: 0,
				limit: 10
			})

			expect(forwardSpy).not.toHaveBeenCalled()
		})
	})
})

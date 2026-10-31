// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { beforeEach, describe, expect, it, vi } from 'vitest'
import usersApi from '@/app/api/users.api'
import { httpService } from '@/app/services/http.service'
import statsHandler from '@@/server/api/users/stats.get'
import summaryHandler from '@@/server/api/users/summary.get'
import * as clusterService from '@@/server/services/cluster.service'

vi.mock('@/app/services/http.service', () => ({
	httpService: {
		get: vi.fn(),
		post: vi.fn(),
		put: vi.fn(),
		delete: vi.fn()
	}
}))

vi.mock('@@/server/services/cluster.service', () => ({
	getClustersConfig: vi.fn(),
	forwardToCluster: vi.fn(),
	resolveClusterTargets: vi.fn((header, clusters) => {
		if (header) {
			const matched = clusters.find((c: any) => c.name === header || c.url === header)
			return matched ? [matched] : clusters
		}
		return clusters
	})
}))

let mockRequestHeaders: Record<string, string> = { authorization: 'Bearer test-token' }

vi.mock('h3', () => ({
	defineEventHandler: (fn: any) => fn,
	getRequestHeaders: vi.fn(() => ({ authorization: 'Bearer test-token' })),
	getRequestHeader: vi.fn((_event: any, name: string) => mockRequestHeaders[name])
}))

describe('User Summary / Stats Feature', () => {
	beforeEach(() => {
		vi.clearAllMocks()
		mockRequestHeaders = { authorization: 'Bearer test-token' }
	})

	describe('usersApi.getUserSummary & getUserStats', () => {
		it('should call httpService.get with /api/users/summary', async () => {
			const mockResponse = {
				status_code: 200,
				success: true,
				data: {
					total_user: 15,
					total_active_user: 12,
					total_disabled_user: 3,
					total_allow_sso: 10,
					user_cluster: {
						'cluster-01': 10,
						'cluster-02': 5
					}
				}
			}
			vi.mocked(httpService.get).mockResolvedValueOnce(mockResponse as any)

			const result = await usersApi.getUserSummary()

			expect(httpService.get).toHaveBeenCalledWith('/api/users/summary', undefined, undefined)
			expect(result).toEqual(mockResponse)
		})

		it('should delegate getUserStats to getUserSummary', async () => {
			const mockResponse = {
				status_code: 200,
				success: true,
				data: {
					total_user: 5,
					total_active_user: 4,
					total_disabled_user: 1,
					total_allow_sso: 3,
					user_cluster: { patrolli1: 5 }
				}
			}
			vi.mocked(httpService.get).mockResolvedValueOnce(mockResponse as any)

			const result = await usersApi.getUserStats(['patrolli1'])

			expect(httpService.get).toHaveBeenCalledWith('/api/users/summary', undefined, [
				'patrolli1'
			])
			expect(result).toEqual(mockResponse)
		})
	})

	describe('BFF GET /api/users/summary handler', () => {
		it('should forward to /api/users/summary on each cluster and aggregate totals accurately', async () => {
			const mockClusters = [
				{ name: 'Cluster Alpha', url: 'http://alpha.cluster' },
				{ name: 'Cluster Beta', url: 'http://beta.cluster' }
			]
			vi.mocked(clusterService.getClustersConfig).mockReturnValueOnce(mockClusters as any)

			vi.mocked(clusterService.forwardToCluster)
				.mockResolvedValueOnce({
					data: {
						data: {
							total: 7,
							allow_sso: 5,
							disabled: 2
						}
					}
				} as any)
				.mockResolvedValueOnce({
					data: {
						data: {
							total: 3,
							allow_sso: 3,
							disabled: 0
						}
					}
				} as any)

			const event = {} as any
			const result = await summaryHandler(event)

			expect(clusterService.forwardToCluster).toHaveBeenCalledWith(
				mockClusters[0],
				'/api/users/summary',
				expect.objectContaining({ method: 'GET' })
			)
			expect(clusterService.forwardToCluster).toHaveBeenCalledWith(
				mockClusters[1],
				'/api/users/summary',
				expect.objectContaining({ method: 'GET' })
			)

			expect(result).toEqual({
				status_code: 200,
				success: true,
				data: {
					total_user: 10,
					total_active_user: 8, // (7-2) + (3-0)
					total_disabled_user: 2, // 2 + 0
					total_allow_sso: 8, // 5 + 3
					user_cluster: {
						'Cluster Alpha': 7,
						'Cluster Beta': 3
					}
				}
			})
		})

		it('should respect x-cluster-url header and target single cluster when requested', async () => {
			const mockClusters = [
				{ name: 'Cluster Alpha', url: 'http://alpha.cluster' },
				{ name: 'Cluster Beta', url: 'http://beta.cluster' }
			]
			vi.mocked(clusterService.getClustersConfig).mockReturnValueOnce(mockClusters as any)
			mockRequestHeaders['x-cluster-url'] = 'Cluster Beta'

			vi.mocked(clusterService.forwardToCluster).mockResolvedValueOnce({
				data: {
					data: {
						total: 4,
						allow_sso: 4,
						disabled: 1
					}
				}
			} as any)

			const event = {} as any
			const result = await summaryHandler(event)

			expect(clusterService.forwardToCluster).toHaveBeenCalledTimes(1)
			expect(clusterService.forwardToCluster).toHaveBeenCalledWith(
				mockClusters[1],
				'/api/users/summary',
				expect.objectContaining({ method: 'GET' })
			)

			expect(result).toEqual({
				status_code: 200,
				success: true,
				data: {
					total_user: 4,
					total_active_user: 3, // 4 - 1
					total_disabled_user: 1,
					total_allow_sso: 4,
					user_cluster: {
						'Cluster Beta': 4
					}
				}
			})
		})

		it('should handle cluster errors gracefully and default that cluster count to 0', async () => {
			const mockClusters = [
				{ name: 'Cluster Alpha', url: 'http://alpha.cluster' },
				{ name: 'Cluster Broken', url: 'http://broken.cluster' }
			]
			vi.mocked(clusterService.getClustersConfig).mockReturnValueOnce(mockClusters as any)

			vi.mocked(clusterService.forwardToCluster)
				.mockResolvedValueOnce({
					data: {
						data: {
							total: 5,
							allow_sso: 3,
							disabled: 1
						}
					}
				} as any)
				.mockRejectedValueOnce(new Error('Connection refused'))

			const event = {} as any
			const result = await summaryHandler(event)

			expect(result).toEqual({
				status_code: 200,
				success: true,
				data: {
					total_user: 5,
					total_active_user: 4,
					total_disabled_user: 1,
					total_allow_sso: 3,
					user_cluster: {
						'Cluster Alpha': 5,
						'Cluster Broken': 0
					}
				}
			})
		})

		it('should return 0 totals when no clusters configured', async () => {
			vi.mocked(clusterService.getClustersConfig).mockReturnValueOnce([])

			const event = {} as any
			const result = await summaryHandler(event)

			expect(result).toEqual({
				status_code: 200,
				success: true,
				data: {
					total_user: 0,
					total_active_user: 0,
					total_disabled_user: 0,
					total_allow_sso: 0,
					user_cluster: {}
				}
			})
		})

		it('should delegate statsHandler directly to summaryHandler', async () => {
			vi.mocked(clusterService.getClustersConfig).mockReturnValueOnce([])

			const event = {} as any
			const result = await statsHandler(event)

			expect(result).toEqual({
				status_code: 200,
				success: true,
				data: {
					total_user: 0,
					total_active_user: 0,
					total_disabled_user: 0,
					total_allow_sso: 0,
					user_cluster: {}
				}
			})
		})
	})
})

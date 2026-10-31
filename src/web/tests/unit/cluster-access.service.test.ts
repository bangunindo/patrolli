// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { verifyUserClusterAccess } from '@@/server/services/cluster-access.service'
import * as clusterService from '@@/server/services/cluster.service'
import * as authCookies from '@@/server/utils/auth-cookies'
import appHelper from '@/app/helpers/app.helper'
import cookieHelper from '@/app/helpers/cookie.helper'

vi.mock('@@/server/services/cluster.service', () => ({
	getClustersConfig: vi.fn(),
	forwardToCluster: vi.fn()
}))

vi.mock('@@/server/utils/auth-cookies', () => ({
	setUserClustersCookie: vi.fn(),
	clearAuthCookies: vi.fn(),
	setAuthCookies: vi.fn()
}))

vi.mock('@/app/helpers/cookie.helper', () => ({
	cookieHelper: {
		get: vi.fn(),
		set: vi.fn(),
		remove: vi.fn(),
		clearAll: vi.fn()
	},
	default: {
		get: vi.fn(),
		set: vi.fn(),
		remove: vi.fn(),
		clearAll: vi.fn()
	}
}))

describe('Cluster Access Verification Feature', () => {
	const mockEvent = {} as any

	beforeEach(() => {
		vi.clearAllMocks()
	})

	describe('verifyUserClusterAccess service', () => {
		it('should return empty list and set empty cookie when accessToken is missing', async () => {
			const result = await verifyUserClusterAccess(mockEvent, '')

			expect(result).toEqual([])
			expect(authCookies.setUserClustersCookie).toHaveBeenCalledWith(mockEvent, [])
		})

		it('should return empty list and set empty cookie when no clusters configured', async () => {
			vi.mocked(clusterService.getClustersConfig).mockReturnValueOnce([])

			const result = await verifyUserClusterAccess(mockEvent, 'valid-token')

			expect(result).toEqual([])
			expect(authCookies.setUserClustersCookie).toHaveBeenCalledWith(mockEvent, [])
		})

		it('should query /api/nodes on all clusters and filter only clusters returning HTTP 200', async () => {
			const mockClusters = [
				{
					key: 'patrolli',
					name: 'patrolli',
					url: 'http://patrolli:8080',
					clientPath: '/patrolli'
				},
				{
					key: 'patrolli1',
					name: 'patrolli1',
					url: 'http://patrolli1:8080',
					clientPath: '/patrolli1'
				},
				{
					key: 'patrolli2',
					name: 'patrolli2',
					url: 'http://patrolli2:8080',
					clientPath: '/patrolli2'
				}
			]
			vi.mocked(clusterService.getClustersConfig).mockReturnValueOnce(mockClusters as any)

			vi.mocked(clusterService.forwardToCluster).mockImplementation(async (cluster: any) => {
				if (cluster.name === 'patrolli') {
					return { status: 200, headers: new Headers(), data: { nodes: [] } }
				}
				if (cluster.name === 'patrolli1') {
					return { status: 401, headers: new Headers(), data: { error: 'Unauthorized' } }
				}
				throw new Error('Connection refused')
			})

			const result = await verifyUserClusterAccess(mockEvent, 'token-123')

			expect(clusterService.forwardToCluster).toHaveBeenCalledTimes(3)
			expect(clusterService.forwardToCluster).toHaveBeenCalledWith(
				mockClusters[0],
				'/api/nodes',
				{
					method: 'GET',
					headers: {
						authorization: 'Bearer token-123'
					}
				}
			)

			expect(result).toHaveLength(1)
			expect(result[0]).toEqual({
				key: 'patrolli',
				name: 'patrolli',
				url: '/patrolli',
				clientPath: '/patrolli',
				label: 'patrolli',
				value: '/patrolli'
			})

			expect(authCookies.setUserClustersCookie).toHaveBeenCalledWith(mockEvent, result)
		})
	})

	describe('appHelper.getBaseURLs integration', () => {
		it('should return cluster items parsed from user_clusters cookie when present', () => {
			const cookieData = JSON.stringify([
				{
					key: 'cluster-alpha',
					name: 'cluster-alpha',
					url: '/cluster-alpha',
					clientPath: '/cluster-alpha',
					label: 'cluster-alpha',
					value: '/cluster-alpha'
				}
			])
			vi.mocked(cookieHelper.get).mockReturnValue(cookieData)

			const urls = appHelper.getBaseURLs()

			expect(urls).toEqual([
				{
					alias: 'cluster-alpha',
					name: 'cluster-alpha',
					label: 'cluster-alpha',
					value: '/cluster-alpha'
				}
			])
		})

		it('should fallback to default if user_clusters cookie is null', () => {
			vi.mocked(cookieHelper.get).mockReturnValue(null)

			const urls = appHelper.getBaseURLs()

			expect(urls).toBeDefined()
			expect(Array.isArray(urls)).toBe(true)
		})
	})
})

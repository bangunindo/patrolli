// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { handleGetClusters } from '@@/server/modules/clusters/clusters.controller'
import * as authRefreshService from '@@/server/services/auth-refresh.service'
import { verifyUserClusterAccess } from '@@/server/services/cluster-access.service'
import * as clusterService from '@@/server/services/cluster.service'
import * as authCookies from '@@/server/utils/auth-cookies'
import appHelper from '@/app/helpers/app.helper'
import cookieHelper from '@/app/helpers/cookie.helper'

vi.mock('@@/server/services/cluster.service', () => ({
	getClustersConfig: vi.fn(),
	forwardToCluster: vi.fn()
}))

vi.mock('@@/server/services/auth-refresh.service', () => ({
	tryTransparentRefreshOn401: vi.fn()
}))

vi.mock('@@/server/utils/auth-cookies', () => ({
	setUserClustersCookie: vi.fn(),
	clearAuthCookies: vi.fn(),
	setAuthCookies: vi.fn(),
	getAuthCookies: vi.fn(),
	isJwtExpired: vi.fn()
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
			vi.mocked(clusterService.getClustersConfig).mockReturnValue(mockClusters as any)

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
				'/api/v1/nodes',
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

	describe('handleGetClusters pre-emptive and transparent refresh', () => {
		it('should refresh expired access_token via tryTransparentRefreshOn401 before verifying cluster access', async () => {
			const mockClusters = [
				{
					key: 'patrolli',
					name: 'patrolli',
					url: 'http://patrolli:8080',
					clientPath: '/patrolli'
				}
			]
			vi.mocked(clusterService.getClustersConfig).mockReturnValue(mockClusters as any)
			vi.mocked(authCookies.getAuthCookies).mockReturnValueOnce({
				access_token: 'expired-token',
				refresh_token: 'valid-rt',
				id_token: undefined,
				auth_provider: 'local',
				is_authenticated: true,
				active_cluster_url: '/patrolli'
			})
			vi.mocked(authCookies.isJwtExpired).mockReturnValueOnce(true)
			vi.mocked(authRefreshService.tryTransparentRefreshOn401).mockResolvedValueOnce(
				'fresh-token-abc'
			)
			vi.mocked(clusterService.forwardToCluster).mockResolvedValueOnce({
				status: 200,
				headers: new Headers(),
				data: { nodes: [] }
			})

			const event = {
				path: '/api/clusters?refresh=true',
				node: {
					req: {
						url: '/api/clusters?refresh=true',
						headers: {}
					},
					res: {}
				}
			} as any

			const res = await handleGetClusters(event)

			expect(authRefreshService.tryTransparentRefreshOn401).toHaveBeenCalledWith(
				event,
				'/patrolli'
			)
			expect(clusterService.forwardToCluster).toHaveBeenCalledWith(
				mockClusters[0],
				'/api/v1/nodes',
				{
					method: 'GET',
					headers: {
						authorization: 'Bearer fresh-token-abc'
					}
				}
			)
			expect(res.success).toBe(true)
			expect(res.data).toHaveLength(1)
		})

		it('should transparently refresh when cluster access check returns 401 for an unexpired token', async () => {
			const mockClusters = [
				{
					key: 'patrolli',
					name: 'patrolli',
					url: 'http://patrolli:8080',
					clientPath: '/patrolli'
				}
			]
			vi.mocked(clusterService.getClustersConfig).mockReturnValue(mockClusters as any)
			vi.mocked(authCookies.getAuthCookies).mockReturnValueOnce({
				access_token: 'revoked-token',
				refresh_token: 'valid-rt',
				id_token: undefined,
				auth_provider: 'local',
				is_authenticated: true,
				active_cluster_url: '/patrolli'
			})
			vi.mocked(authCookies.isJwtExpired).mockReturnValueOnce(false)
			vi.mocked(clusterService.forwardToCluster)
				.mockResolvedValueOnce({
					status: 401,
					headers: new Headers(),
					data: { error: 'Unauthorized' }
				})
				.mockResolvedValueOnce({
					status: 200,
					headers: new Headers(),
					data: { nodes: [] }
				})
			vi.mocked(authRefreshService.tryTransparentRefreshOn401).mockResolvedValueOnce(
				'fresh-token-401'
			)

			const event = {
				path: '/api/clusters?refresh=true',
				node: { req: { url: '/api/clusters?refresh=true', headers: {} }, res: {} }
			} as any

			const res = await handleGetClusters(event)

			expect(authRefreshService.tryTransparentRefreshOn401).toHaveBeenCalledTimes(1)
			expect(clusterService.forwardToCluster).toHaveBeenCalledTimes(2)
			expect(res.success).toBe(true)
			expect(res.data).toHaveLength(1)
		})

		it('should not refresh when cluster access check returns 403 without any 401', async () => {
			const mockClusters = [
				{
					key: 'patrolli',
					name: 'patrolli',
					url: 'http://patrolli:8080',
					clientPath: '/patrolli'
				}
			]
			vi.mocked(clusterService.getClustersConfig).mockReturnValue(mockClusters as any)
			vi.mocked(authCookies.getAuthCookies).mockReturnValueOnce({
				access_token: 'valid-token-no-access',
				refresh_token: 'valid-rt',
				id_token: undefined,
				auth_provider: 'local',
				is_authenticated: true,
				active_cluster_url: '/patrolli'
			})
			vi.mocked(authCookies.isJwtExpired).mockReturnValueOnce(false)
			vi.mocked(clusterService.forwardToCluster).mockResolvedValueOnce({
				status: 403,
				headers: new Headers(),
				data: { error: 'Forbidden' }
			})

			const event = {
				path: '/api/clusters?refresh=true',
				node: { req: { url: '/api/clusters?refresh=true', headers: {} }, res: {} }
			} as any

			const res = await handleGetClusters(event)

			expect(authRefreshService.tryTransparentRefreshOn401).not.toHaveBeenCalled()
			expect(clusterService.forwardToCluster).toHaveBeenCalledTimes(1)
			expect(res.success).toBe(true)
			expect(res.data).toHaveLength(1)
		})
	})
})

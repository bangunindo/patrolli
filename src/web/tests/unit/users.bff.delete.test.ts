// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { beforeEach, describe, expect, it, vi } from 'vitest'
import deleteUserHandler from '@@/server/api/users/[userId].delete'
import * as clusterService from '@@/server/services/cluster.service'

vi.mock('@@/server/services/cluster.service', () => ({
	getClustersConfig: vi.fn(),
	resolveClusterTargets: vi.fn(),
	forwardToCluster: vi.fn()
}))

vi.mock('h3', () => ({
	defineEventHandler: (fn: any) => fn,
	getRequestHeaders: vi.fn(() => ({})),
	getQuery: vi.fn(() => ({})),
	setResponseStatus: vi.fn()
}))

describe('BFF DELETE /api/users/:userId - Multi-Cluster Resolution', () => {
	beforeEach(() => {
		vi.clearAllMocks()
	})

	it('should delete user directly with userId for single target cluster', async () => {
		const clusterAlpha = { name: 'Cluster Alpha', clientPath: '/alpha', url: 'http://alpha' }
		vi.mocked(clusterService.getClustersConfig).mockReturnValueOnce([clusterAlpha as any])
		vi.mocked(clusterService.resolveClusterTargets).mockReturnValueOnce([clusterAlpha as any])

		vi.mocked(clusterService.forwardToCluster).mockResolvedValueOnce({
			status: 200,
			headers: new Headers(),
			data: { data: { message: 'Deleted' } }
		})

		const event = {
			context: {
				params: { userId: 'alpha-1' }
			}
		} as any

		const res = await deleteUserHandler(event)

		expect(clusterService.forwardToCluster).toHaveBeenCalledWith(
			clusterAlpha,
			'/api/users/alpha-1',
			expect.objectContaining({ method: 'DELETE' })
		)
		expect(res.status_code).toBe(200)
		expect(res.success).toBe(true)
	})

	it('should resolve username, find local user_id on each cluster, delete matching clusters, and skip clusters where user does not exist', async () => {
		const clusterAlpha = { name: 'Cluster Alpha', clientPath: '/alpha', url: 'http://alpha' }
		const clusterBeta = { name: 'Cluster Beta', clientPath: '/beta', url: 'http://beta' }

		vi.mocked(clusterService.getClustersConfig).mockReturnValue([
			clusterAlpha,
			clusterBeta
		] as any)
		vi.mocked(clusterService.resolveClusterTargets).mockReturnValueOnce([
			clusterAlpha,
			clusterBeta
		] as any)

		vi.mocked(clusterService.forwardToCluster).mockImplementation(
			async (cluster, targetPath, options) => {
				// 1. Initial lookup to get username by userId
				if (targetPath === '/api/users/repr-id' && options.method === 'GET') {
					return {
						status: 200,
						headers: new Headers(),
						data: { data: { user_id: 'repr-id', username: 'john' } }
					}
				}
				// 2. Search users by username
				if (targetPath === '/api/users' && options.method === 'GET') {
					if (cluster.name === 'Cluster Alpha') {
						return {
							status: 200,
							headers: new Headers(),
							data: { data: { users: [{ user_id: 'alpha-john', username: 'john' }] } }
						}
					}
					// Beta does not have john
					return { status: 200, headers: new Headers(), data: { data: { users: [] } } }
				}
				// 3. Delete user
				if (options.method === 'DELETE') {
					return {
						status: 200,
						headers: new Headers(),
						data: { data: { message: 'Deleted' } }
					}
				}
				return { status: 404, headers: new Headers(), data: {} }
			}
		)

		const event = {
			context: {
				params: { userId: 'repr-id' }
			}
		} as any

		const res = await deleteUserHandler(event)

		// Alpha had john -> should delete /api/users/alpha-john
		expect(clusterService.forwardToCluster).toHaveBeenCalledWith(
			clusterAlpha,
			'/api/users/alpha-john',
			expect.objectContaining({ method: 'DELETE' })
		)

		// Beta did not have john -> should NOT delete on Beta
		expect(clusterService.forwardToCluster).not.toHaveBeenCalledWith(
			clusterBeta,
			expect.stringMatching(/\/api\/users\//),
			expect.objectContaining({ method: 'DELETE' })
		)

		expect(res.status_code).toBe(200)
		expect(res.success).toBe(true)
	})

	it('should return 404 if user not found in any of the target clusters', async () => {
		const clusterAlpha = { name: 'Cluster Alpha', clientPath: '/alpha', url: 'http://alpha' }
		const clusterBeta = { name: 'Cluster Beta', clientPath: '/beta', url: 'http://beta' }

		vi.mocked(clusterService.getClustersConfig).mockReturnValue([
			clusterAlpha,
			clusterBeta
		] as any)
		vi.mocked(clusterService.resolveClusterTargets).mockReturnValueOnce([
			clusterAlpha,
			clusterBeta
		] as any)

		vi.mocked(clusterService.forwardToCluster).mockImplementation(
			async (cluster, targetPath, options) => {
				if (options.method === 'GET') {
					return { status: 404, headers: new Headers(), data: { data: { users: [] } } }
				}
				return { status: 404, headers: new Headers(), data: {} }
			}
		)

		const event = {
			context: {
				params: { userId: 'ghost-id' }
			}
		} as any

		const res = await deleteUserHandler(event)

		expect(res.status_code).toBe(404)
		expect(res.success).toBe(false)
	})
})

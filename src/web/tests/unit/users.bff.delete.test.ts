// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { beforeEach, describe, expect, it, vi } from 'vitest'
import deleteUserHandler from '@@/server/api/users/[userId].delete'
import * as clusterService from '@@/server/services/cluster.service'
import * as h3 from 'h3'

vi.mock('@@/server/services/cluster.service', () => ({
	getClustersConfig: vi.fn(),
	resolveClusterTargets: vi.fn(),
	forwardToCluster: vi.fn()
}))

vi.mock('h3', () => ({
	defineEventHandler: (fn: any) => fn,
	deleteCookie: vi.fn(),
	getCookie: vi.fn(() => undefined),
	getRequestHeader: vi.fn(() => undefined),
	getRequestHeaders: vi.fn(() => ({})),
	getQuery: vi.fn(() => ({})),
	parseCookies: vi.fn(() => ({})),
	setCookie: vi.fn(),
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
			'/api/v1/users/alpha-1',
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
		vi.mocked(clusterService.resolveClusterTargets).mockReturnValue([
			clusterAlpha,
			clusterBeta
		] as any)
		vi.mocked(h3.getCookie).mockImplementation((_event, name) => {
			if (name === 'access_token') return 'non-expired-but-revoked'
			if (name === 'refresh_token') return 'rt-1'
			if (name === 'auth_provider') return 'local'
			return undefined
		})

		vi.mocked(clusterService.forwardToCluster).mockImplementation(
			async (cluster, targetPath, options) => {
				// 1. Initial lookup to get username by userId
				if (targetPath === '/api/v1/users/repr-id' && options.method === 'GET') {
					return {
						status: 200,
						headers: new Headers(),
						data: { data: { user_id: 'repr-id', username: 'john' } }
					}
				}
				// 2. Search users by username
				if (targetPath === '/api/v1/users' && options.method === 'GET') {
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

		// Alpha had john -> should delete /api/v1/users/alpha-john
		expect(clusterService.forwardToCluster).toHaveBeenCalledWith(
			clusterAlpha,
			'/api/v1/users/alpha-john',
			expect.objectContaining({ method: 'DELETE' })
		)

		// Beta did not have john -> should NOT delete on Beta
		expect(clusterService.forwardToCluster).not.toHaveBeenCalledWith(
			clusterBeta,
			expect.stringMatching(/\/api\/v1\/users\//),
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
		vi.mocked(clusterService.resolveClusterTargets).mockReturnValue([
			clusterAlpha,
			clusterBeta
		] as any)
		vi.mocked(h3.getCookie).mockImplementation((_event, name) => {
			if (name === 'access_token') return 'non-expired-but-revoked'
			if (name === 'refresh_token') return 'rt-1'
			if (name === 'auth_provider') return 'local'
			return undefined
		})

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

	it('should refresh and retry when resolveUsernameAcrossClusters receives 401', async () => {
		const clusterAlpha = { name: 'Cluster Alpha', clientPath: '/alpha', url: 'http://alpha' }
		const clusterBeta = { name: 'Cluster Beta', clientPath: '/beta', url: 'http://beta' }

		vi.mocked(clusterService.getClustersConfig).mockReturnValue([
			clusterAlpha,
			clusterBeta
		] as any)
		vi.mocked(clusterService.resolveClusterTargets).mockReturnValue([
			clusterAlpha,
			clusterBeta
		] as any)
		vi.mocked(h3.getCookie).mockImplementation((_event, name) => {
			if (name === 'access_token') return 'non-expired-but-revoked'
			if (name === 'refresh_token') return 'rt-1'
			if (name === 'auth_provider') return 'local'
			return undefined
		})

		let lookupAttempt = 0
		vi.mocked(clusterService.forwardToCluster).mockImplementation(
			async (cluster, targetPath, options) => {
				if (targetPath === '/api/v1/auth/refresh') {
					return {
						status: 200,
						headers: new Headers(),
						data: {
							data: {
								access_token: 'refreshed-jwt',
								refresh_token: 'refreshed-rt'
							}
						}
					}
				}
				if (targetPath === '/api/v1/users/repr-id' && options.method === 'GET') {
					lookupAttempt++
					if (lookupAttempt <= 2) {
						return { status: 401, headers: new Headers(), data: {} }
					}
					return {
						status: 200,
						headers: new Headers(),
						data: { data: { username: 'john' } }
					}
				}
				if (targetPath === '/api/v1/users' && options.method === 'GET') {
					if (cluster.name === 'Cluster Alpha') {
						return {
							status: 200,
							headers: new Headers(),
							data: { data: { users: [{ user_id: 'alpha-john', username: 'john' }] } }
						}
					}
					return { status: 200, headers: new Headers(), data: { data: { users: [] } } }
				}
				if (targetPath === '/api/v1/users/alpha-john' && options.method === 'DELETE') {
					return { status: 200, headers: new Headers(), data: { success: true } }
				}
				return { status: 404, headers: new Headers(), data: {} }
			}
		)

		const event = {
			node: {
				req: {
					headers: {
						cookie: 'access_token=non-expired-but-revoked; refresh_token=rt-1; auth_provider=local'
					}
				},
				res: {
					getHeader: vi.fn(),
					setHeader: vi.fn()
				}
			},
			context: {
				params: { userId: 'repr-id' }
			}
		} as any

		const res = await deleteUserHandler(event)

		expect(res.status_code).toBe(200)
		expect(res.success).toBe(true)
		expect(clusterService.forwardToCluster).toHaveBeenCalledWith(
			clusterAlpha,
			'/api/v1/users/alpha-john',
			expect.objectContaining({
				method: 'DELETE',
				headers: expect.objectContaining({ authorization: 'Bearer refreshed-jwt' })
			})
		)
	})

	it('strips uppercase Authorization header when retrying username resolution after 401 refresh', async () => {
		let lookupAttempt = 0
		vi.spyOn(clusterService, 'forwardToCluster').mockImplementation(
			async (cluster, targetPath, options) => {
				if (targetPath === '/api/v1/auth/refresh') {
					return {
						status: 200,
						headers: new Headers(),
						data: {
							data: {
								access_token: 'refreshed-jwt',
								refresh_token: 'refreshed-rt'
							}
						}
					}
				}
				if (targetPath === '/api/v1/users/repr-id' && options.method === 'GET') {
					lookupAttempt++
					if (lookupAttempt <= 2) {
						return { status: 401, headers: new Headers(), data: {} }
					}
					expect(options.headers?.Authorization).toBeUndefined()
					expect(options.headers?.authorization).toBe('Bearer refreshed-jwt')
					return {
						status: 200,
						headers: new Headers(),
						data: { data: { username: 'john' } }
					}
				}
				if (targetPath === '/api/v1/users' && options.method === 'GET') {
					if (cluster.name === 'Cluster Alpha') {
						return {
							status: 200,
							headers: new Headers(),
							data: { data: { users: [{ user_id: 'alpha-john', username: 'john' }] } }
						}
					}
					return { status: 200, headers: new Headers(), data: { data: { users: [] } } }
				}
				if (targetPath === '/api/v1/users/alpha-john' && options.method === 'DELETE') {
					return { status: 200, headers: new Headers(), data: { success: true } }
				}
				return { status: 404, headers: new Headers(), data: {} }
			}
		)

		const event = {
			node: {
				req: {
					headers: {
						Authorization: 'Bearer stale-jwt',
						cookie: 'refresh_token=rt-1; auth_provider=local'
					}
				},
				res: {
					getHeader: vi.fn(),
					setHeader: vi.fn()
				}
			},
			context: {
				params: { userId: 'repr-id' }
			}
		} as any

		const res = await deleteUserHandler(event)

		expect(res.status_code).toBe(200)
		expect(res.success).toBe(true)
	})

	it('does not trigger a second refresh in resolveUsernameAcrossClustersWithAuthRetry when proactive refresh already ran', async () => {
		const clusterAlpha = { name: 'Cluster Alpha', clientPath: '/alpha', url: 'http://alpha' }
		const clusterBeta = { name: 'Cluster Beta', clientPath: '/beta', url: 'http://beta' }

		vi.mocked(clusterService.getClustersConfig).mockReturnValue([
			clusterAlpha,
			clusterBeta
		] as any)
		vi.mocked(clusterService.resolveClusterTargets).mockReturnValue([
			clusterAlpha,
			clusterBeta
		] as any)

		const expiredJwt = `hdr.${Buffer.from(
			JSON.stringify({ exp: Math.floor(Date.now() / 1000) - 60 })
		).toString('base64url')}.sig`
		const freshJwt = `hdr.${Buffer.from(
			JSON.stringify({ exp: Math.floor(Date.now() / 1000) + 300 })
		).toString('base64url')}.sig`

		vi.mocked(h3.getCookie).mockImplementation((_event, name) => {
			if (name === 'access_token') return expiredJwt
			if (name === 'refresh_token') return 'rt-single-use'
			if (name === 'auth_provider') return 'local'
			return undefined
		})

		let refreshCalls = 0
		vi.mocked(clusterService.forwardToCluster).mockImplementation(
			async (_cluster, targetPath, options) => {
				if (targetPath === '/api/v1/auth/refresh') {
					refreshCalls++
					return {
						status: 200,
						headers: new Headers(),
						data: {
							data: {
								access_token: freshJwt,
								refresh_token: 'rt-rotated'
							}
						}
					}
				}
				if (targetPath === '/api/v1/users/repr-id' && options.method === 'GET') {
					return { status: 401, headers: new Headers(), data: {} }
				}
				return { status: 404, headers: new Headers(), data: {} }
			}
		)

		const event = {
			node: {
				req: {
					headers: {
						cookie: `access_token=${expiredJwt}; refresh_token=rt-single-use; auth_provider=local`
					}
				},
				res: {
					getHeader: vi.fn(),
					setHeader: vi.fn()
				}
			},
			context: {
				params: { userId: 'repr-id' }
			}
		} as any

		await deleteUserHandler(event)

		// Proactive refresh runs once in resolveEffectiveRequestHeaders (2 clusters in parallel = 2 cluster calls, 1 refresh batch)
		// Neither resolveUsernameAcrossClustersWithAuthRetry nor executeClusterRequestsWithAuthRetry should trigger a second refresh
		expect(refreshCalls).toBe(2)
		expect(event.context._refreshedAccessToken).toBe(freshJwt)
	})
})

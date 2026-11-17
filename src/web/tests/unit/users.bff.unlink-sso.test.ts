// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { beforeEach, describe, expect, it, vi } from 'vitest'
import unlinkSsoHandler from '@@/server/api/users/[userId]/unlink-sso.post'
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
	readBody: vi.fn(),
	setCookie: vi.fn(),
	setResponseStatus: vi.fn()
}))

describe('BFF POST /api/users/:userId/unlink-sso - Multi-Cluster Unlink SSO', () => {
	const clusterAlpha = { name: 'Cluster Alpha', clientPath: '/alpha', url: 'http://alpha' }
	const clusterBeta = { name: 'Cluster Beta', clientPath: '/beta', url: 'http://beta' }
	const clusterGamma = { name: 'Cluster Gamma', clientPath: '/gamma', url: 'http://gamma' }

	beforeEach(() => {
		vi.clearAllMocks()
	})

	it('should unlink SSO on each cluster where user exists by username and skip non-matching clusters', async () => {
		vi.mocked(clusterService.getClustersConfig).mockReturnValueOnce([
			clusterAlpha,
			clusterBeta,
			clusterGamma
		] as any)
		vi.mocked(clusterService.resolveClusterTargets).mockReturnValueOnce([
			clusterAlpha,
			clusterBeta,
			clusterGamma
		] as any)

		vi.mocked(h3.readBody).mockResolvedValueOnce({
			username: 'alice'
		})

		vi.mocked(clusterService.forwardToCluster).mockImplementation(
			async (cluster, targetPath, options) => {
				if (cluster.name === 'Cluster Alpha') {
					if (options.method === 'GET') {
						return {
							status: 200,
							headers: new Headers(),
							data: { data: { users: [{ user_id: 'alpha-1', username: 'alice' }] } }
						}
					}
					if (options.method === 'POST') {
						return {
							status: 200,
							headers: new Headers(),
							data: { success: true }
						}
					}
				}
				if (cluster.name === 'Cluster Beta') {
					if (options.method === 'GET') {
						return {
							status: 200,
							headers: new Headers(),
							data: { data: { users: [{ user_id: 'beta-1', username: 'alice' }] } }
						}
					}
					if (options.method === 'POST') {
						return {
							status: 200,
							headers: new Headers(),
							data: { success: true }
						}
					}
				}
				if (cluster.name === 'Cluster Gamma') {
					if (options.method === 'GET') {
						return {
							status: 200,
							headers: new Headers(),
							data: { data: { users: [] } }
						}
					}
				}
				return { status: 404, headers: new Headers(), data: {} }
			}
		)

		const event = {
			context: {
				params: { userId: 'repr-user-id' }
			}
		} as any

		const res = await unlinkSsoHandler(event)

		// Alpha had alice -> POST /api/v1/users/alpha-1/unlink-sso
		expect(clusterService.forwardToCluster).toHaveBeenCalledWith(
			clusterAlpha,
			'/api/v1/users/alpha-1/unlink-sso',
			expect.objectContaining({ method: 'POST' })
		)

		// Beta had alice -> POST /api/v1/users/beta-1/unlink-sso
		expect(clusterService.forwardToCluster).toHaveBeenCalledWith(
			clusterBeta,
			'/api/v1/users/beta-1/unlink-sso',
			expect.objectContaining({ method: 'POST' })
		)

		// Gamma did NOT have alice -> should NOT call POST on Gamma
		expect(clusterService.forwardToCluster).not.toHaveBeenCalledWith(
			clusterGamma,
			expect.stringMatching(/\/unlink-sso/),
			expect.objectContaining({ method: 'POST' })
		)

		expect(res.status_code).toBe(200)
		expect(res.success).toBe(true)
		expect(res.data.clusters).toEqual(['Cluster Alpha', 'Cluster Beta'])
	})

	it('should return 404 if user is not found in any target cluster', async () => {
		vi.mocked(clusterService.getClustersConfig).mockReturnValueOnce([clusterAlpha] as any)
		vi.mocked(clusterService.resolveClusterTargets).mockReturnValueOnce([clusterAlpha] as any)
		vi.mocked(h3.readBody).mockResolvedValueOnce({ username: 'ghostuser' })

		vi.mocked(clusterService.forwardToCluster).mockResolvedValue({
			status: 200,
			headers: new Headers(),
			data: { data: { users: [] } }
		})

		const event = {
			context: {
				params: { userId: 'ghost-id' }
			}
		} as any

		const res = await unlinkSsoHandler(event)

		expect(res.status_code).toBe(404)
		expect(res.success).toBe(false)
	})

	it('should refresh and retry when resolveUsernameAcrossClusters receives 401', async () => {
		vi.mocked(clusterService.getClustersConfig).mockReturnValue([clusterAlpha] as any)
		vi.mocked(clusterService.resolveClusterTargets).mockReturnValue([clusterAlpha] as any)
		vi.mocked(h3.readBody).mockResolvedValueOnce({})
		vi.mocked(h3.getCookie).mockImplementation((_event, name) => {
			if (name === 'access_token') return 'revoked-jwt'
			if (name === 'refresh_token') return 'rt-1'
			if (name === 'auth_provider') return 'local'
			return undefined
		})

		let lookupAttempt = 0
		vi.mocked(clusterService.forwardToCluster).mockImplementation(
			async (_cluster, targetPath, options) => {
				if (targetPath === '/api/v1/auth/refresh') {
					return {
						status: 200,
						headers: new Headers(),
						data: {
							data: {
								access_token: 'refreshed-unlink-jwt',
								refresh_token: 'refreshed-rt'
							}
						}
					}
				}
				if (targetPath === '/api/v1/users/repr-user-id' && options.method === 'GET') {
					lookupAttempt++
					if (lookupAttempt === 1) {
						return { status: 401, headers: new Headers(), data: {} }
					}
					return {
						status: 200,
						headers: new Headers(),
						data: { data: { username: 'alice' } }
					}
				}
				if (targetPath === '/api/v1/users' && options.method === 'GET') {
					return {
						status: 200,
						headers: new Headers(),
						data: { data: { users: [{ user_id: 'alpha-1', username: 'alice' }] } }
					}
				}
				if (
					targetPath === '/api/v1/users/alpha-1/unlink-sso' &&
					options.method === 'POST'
				) {
					return { status: 200, headers: new Headers(), data: { success: true } }
				}
				return { status: 404, headers: new Headers(), data: {} }
			}
		)

		const event = {
			node: {
				req: {
					headers: {
						cookie: 'access_token=revoked-jwt; refresh_token=rt-1; auth_provider=local'
					}
				},
				res: {
					getHeader: vi.fn(),
					setHeader: vi.fn()
				}
			},
			context: {
				params: { userId: 'repr-user-id' }
			}
		} as any

		const res = await unlinkSsoHandler(event)

		expect(res.status_code).toBe(200)
		expect(res.success).toBe(true)
		expect(clusterService.forwardToCluster).toHaveBeenCalledWith(
			clusterAlpha,
			'/api/v1/users/alpha-1/unlink-sso',
			expect.objectContaining({
				method: 'POST',
				headers: expect.objectContaining({ authorization: 'Bearer refreshed-unlink-jwt' })
			})
		)
	})
})

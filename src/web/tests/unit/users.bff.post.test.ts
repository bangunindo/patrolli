// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { beforeEach, describe, expect, it, vi } from 'vitest'
import postUserHandler from '@@/server/api/users/index.post'
import * as clusterService from '@@/server/services/cluster.service'
import * as h3 from 'h3'

vi.mock('@@/server/services/cluster.service', () => ({
	getClustersConfig: vi.fn(),
	resolveClusterTargets: vi.fn(),
	forwardToCluster: vi.fn()
}))

vi.mock('h3', () => ({
	defineEventHandler: (fn: any) => fn,
	getRequestHeaders: vi.fn(() => ({})),
	getQuery: vi.fn(() => ({})),
	readBody: vi.fn(),
	setResponseStatus: vi.fn()
}))

describe('BFF POST /api/users - Multi-Cluster User Creation', () => {
	beforeEach(() => {
		vi.clearAllMocks()
	})

	it('should create user directly in single target cluster', async () => {
		const clusterAlpha = { name: 'Cluster Alpha', clientPath: '/alpha', url: 'http://alpha' }
		vi.mocked(clusterService.getClustersConfig).mockReturnValueOnce([clusterAlpha as any])
		vi.mocked(clusterService.resolveClusterTargets).mockReturnValueOnce([clusterAlpha as any])

		vi.mocked(h3.readBody).mockResolvedValueOnce({
			username: 'newuser',
			display_name: 'New User',
			password: 'password123',
			adm_status: 1
		})

		vi.mocked(clusterService.forwardToCluster).mockResolvedValueOnce({
			status: 201,
			headers: new Headers(),
			data: {
				data: {
					user_id: 'alpha-new-1',
					username: 'newuser',
					display_name: 'New User',
					adm_status: 1
				}
			}
		})

		const event = {} as any
		const res = await postUserHandler(event)

		expect(clusterService.forwardToCluster).toHaveBeenCalledWith(
			clusterAlpha,
			'/api/users',
			expect.objectContaining({
				method: 'POST',
				body: expect.objectContaining({ username: 'newuser' })
			})
		)
		expect(res.status_code).toBe(201)
		expect(res.data.username).toBe('newuser')
		expect(res.data.clusters).toEqual(['Cluster Alpha'])
	})

	it('should create user in each cluster when multi-cluster target is provided', async () => {
		const clusterAlpha = { name: 'Cluster Alpha', clientPath: '/alpha', url: 'http://alpha' }
		const clusterBeta = { name: 'Cluster Beta', clientPath: '/beta', url: 'http://beta' }

		vi.mocked(clusterService.getClustersConfig).mockReturnValueOnce([
			clusterAlpha,
			clusterBeta
		] as any)
		vi.mocked(clusterService.resolveClusterTargets).mockReturnValueOnce([
			clusterAlpha,
			clusterBeta
		] as any)

		vi.mocked(h3.readBody).mockResolvedValueOnce({
			username: 'alex',
			display_name: 'Alex Turner',
			password: 'password123',
			adm_status: 1
		})

		vi.mocked(clusterService.forwardToCluster).mockImplementation(
			async (cluster, targetPath, options) => {
				if (options.method === 'GET') {
					// Neither cluster has alex yet
					return { status: 200, headers: new Headers(), data: { data: { users: [] } } }
				}
				if (options.method === 'POST') {
					const id = cluster.name === 'Cluster Alpha' ? 'alpha-alex' : 'beta-alex'
					return {
						status: 201,
						headers: new Headers(),
						data: {
							data: {
								user_id: id,
								username: 'alex',
								display_name: 'Alex Turner',
								adm_status: 1
							}
						}
					}
				}
				return { status: 404, headers: new Headers(), data: {} }
			}
		)

		const event = {} as any
		const res = await postUserHandler(event)

		expect(clusterService.forwardToCluster).toHaveBeenCalledWith(
			clusterAlpha,
			'/api/users',
			expect.objectContaining({ method: 'POST' })
		)
		expect(clusterService.forwardToCluster).toHaveBeenCalledWith(
			clusterBeta,
			'/api/users',
			expect.objectContaining({ method: 'POST' })
		)

		expect(res.status_code).toBe(201)
		expect(res.data.username).toBe('alex')
		expect(res.data.clusters).toEqual(['Cluster Alpha', 'Cluster Beta'])
		expect(res.data.cluster_name).toBe('Cluster Alpha, Cluster Beta')
	})

	it('should handle partial success gracefully if user already exists in one cluster', async () => {
		const clusterAlpha = { name: 'Cluster Alpha', clientPath: '/alpha', url: 'http://alpha' }
		const clusterBeta = { name: 'Cluster Beta', clientPath: '/beta', url: 'http://beta' }

		vi.mocked(clusterService.getClustersConfig).mockReturnValueOnce([
			clusterAlpha,
			clusterBeta
		] as any)
		vi.mocked(clusterService.resolveClusterTargets).mockReturnValueOnce([
			clusterAlpha,
			clusterBeta
		] as any)

		vi.mocked(h3.readBody).mockResolvedValueOnce({
			username: 'existinguser',
			display_name: 'Existing User',
			password: 'password123',
			adm_status: 1
		})

		vi.mocked(clusterService.forwardToCluster).mockImplementation(
			async (cluster, targetPath, options) => {
				if (options.method === 'GET') {
					if (cluster.name === 'Cluster Alpha') {
						// Already exists in Alpha
						return {
							status: 200,
							headers: new Headers(),
							data: {
								data: {
									users: [
										{
											user_id: 'alpha-exist',
											username: 'existinguser',
											display_name: 'Existing User'
										}
									]
								}
							}
						}
					}
					// Does NOT exist in Beta
					return { status: 200, headers: new Headers(), data: { data: { users: [] } } }
				}
				if (options.method === 'POST') {
					// Created in Beta
					return {
						status: 201,
						headers: new Headers(),
						data: {
							data: {
								user_id: 'beta-new',
								username: 'existinguser',
								display_name: 'Existing User',
								adm_status: 1
							}
						}
					}
				}
				return { status: 404, headers: new Headers(), data: {} }
			}
		)

		const event = {} as any
		const res = await postUserHandler(event)

		// Alpha should NOT have called POST since user already exists
		expect(clusterService.forwardToCluster).not.toHaveBeenCalledWith(
			clusterAlpha,
			'/api/users',
			expect.objectContaining({ method: 'POST' })
		)

		// Beta SHOULD have called POST to create user
		expect(clusterService.forwardToCluster).toHaveBeenCalledWith(
			clusterBeta,
			'/api/users',
			expect.objectContaining({ method: 'POST' })
		)

		expect(res.status_code).toBe(201)
		expect(res.success).toBe(true)
		expect(res.data.clusters).toContain('Cluster Alpha')
		expect(res.data.clusters).toContain('Cluster Beta')
	})

	it('should return error if creation fails across all target clusters', async () => {
		const clusterAlpha = { name: 'Cluster Alpha', clientPath: '/alpha', url: 'http://alpha' }
		vi.mocked(clusterService.getClustersConfig).mockReturnValueOnce([clusterAlpha as any])
		vi.mocked(clusterService.resolveClusterTargets).mockReturnValueOnce([
			clusterAlpha,
			{ name: 'Cluster Beta', clientPath: '/beta', url: 'http://beta' }
		] as any)

		vi.mocked(h3.readBody).mockResolvedValueOnce({
			username: 'failuser'
		})

		vi.mocked(clusterService.forwardToCluster).mockImplementation(
			async (cluster, targetPath, options) => {
				if (options.method === 'GET') {
					return { status: 200, headers: new Headers(), data: { data: { users: [] } } }
				}
				return { status: 500, headers: new Headers(), data: { message: 'DB Error' } }
			}
		)

		const event = {} as any
		const res = await postUserHandler(event)

		expect(res.status_code).toBe(500)
		expect(res.success).toBe(false)
	})
})

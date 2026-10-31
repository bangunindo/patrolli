// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { beforeEach, describe, expect, it, vi } from 'vitest'
import getUsersHandler from '@@/server/api/users/index.get'
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
	setResponseStatus: vi.fn(),
	setResponseHeaders: vi.fn()
}))

describe('BFF GET /api/users - Deduplication and Multi-Cluster', () => {
	beforeEach(() => {
		vi.clearAllMocks()
	})

	it('should return enriched user with clusters array for single cluster', async () => {
		const mockCluster = { name: 'Cluster Alpha', clientPath: '/alpha', url: 'http://alpha' }
		vi.mocked(clusterService.getClustersConfig).mockReturnValueOnce([mockCluster as any])
		vi.mocked(clusterService.resolveClusterTargets).mockReturnValueOnce([mockCluster as any])

		vi.mocked(clusterService.forwardToCluster).mockResolvedValueOnce({
			status: 200,
			headers: new Headers(),
			data: {
				data: {
					users: [
						{
							user_id: '111',
							username: 'alice',
							display_name: 'Alice',
							adm_status: 1
						}
					],
					total: 1
				}
			}
		})

		const event = {} as any
		const res = await getUsersHandler(event)

		expect(res.status_code).toBe(200)
		expect(res.data.users.length).toBe(1)
		expect(res.data.users[0]).toMatchObject({
			user_id: '111',
			username: 'alice',
			display_name: 'Alice',
			adm_status: 1,
			cluster_name: 'Cluster Alpha',
			cluster_url: '/alpha'
		})
	})

	it('should return all users across multiple clusters without deduplication', async () => {
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

		// Cluster Alpha has 'alice' and 'bob'
		vi.mocked(clusterService.forwardToCluster)
			.mockResolvedValueOnce({
				status: 200,
				headers: new Headers(),
				data: {
					data: {
						users: [
							{
								user_id: 'alpha-1',
								username: 'alice',
								display_name: 'Alice',
								adm_status: 1,
								last_updated: '2026-10-01T00:00:00Z'
							},
							{
								user_id: 'alpha-2',
								username: 'bob',
								display_name: 'Bob',
								adm_status: 1,
								last_updated: '2026-10-02T00:00:00Z'
							}
						],
						total: 2
					}
				}
			})
			// Cluster Beta has 'alice' (different user_id) and 'charlie'
			.mockResolvedValueOnce({
				status: 200,
				headers: new Headers(),
				data: {
					data: {
						users: [
							{
								user_id: 'beta-1',
								username: 'alice',
								display_name: 'Alice',
								adm_status: 1,
								last_updated: '2026-10-05T00:00:00Z'
							},
							{
								user_id: 'beta-2',
								username: 'charlie',
								display_name: 'Charlie',
								adm_status: 1,
								last_updated: '2026-10-03T00:00:00Z'
							}
						],
						total: 2
					}
				}
			})

		const event = {} as any
		const res = await getUsersHandler(event)

		expect(res.status_code).toBe(200)
		expect(res.data.users.length).toBe(4)
		expect(res.data.total).toBe(4)

		const alphaAlice = res.data.users.find(
			(u: any) => u.username === 'alice' && u.user_id === 'alpha-1'
		)
		expect(alphaAlice).toBeDefined()
		expect(alphaAlice.cluster_name).toBe('Cluster Alpha')
		expect(alphaAlice.cluster_url).toBe('/alpha')

		const betaAlice = res.data.users.find(
			(u: any) => u.username === 'alice' && u.user_id === 'beta-1'
		)
		expect(betaAlice).toBeDefined()
		expect(betaAlice.cluster_name).toBe('Cluster Beta')
		expect(betaAlice.cluster_url).toBe('/beta')

		const bob = res.data.users.find((u: any) => u.username === 'bob')
		expect(bob).toBeDefined()
		expect(bob.cluster_name).toBe('Cluster Alpha')

		const charlie = res.data.users.find((u: any) => u.username === 'charlie')
		expect(charlie).toBeDefined()
		expect(charlie.cluster_name).toBe('Cluster Beta')
	})
})

// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { beforeEach, describe, expect, it, vi } from 'vitest'
import patchUserHandler from '@@/server/api/users/[userId].patch'
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

describe('BFF PATCH /api/users/:userId - Single Cluster User Update', () => {
	const clusterAlpha = {
		name: 'Cluster Alpha',
		clientPath: '/alpha',
		url: 'http://alpha',
		key: 'alpha'
	}
	const clusterBeta = {
		name: 'Cluster Beta',
		clientPath: '/beta',
		url: 'http://beta',
		key: 'beta'
	}

	beforeEach(() => {
		vi.clearAllMocks()
	})

	it('should update user directly in single targeted cluster', async () => {
		vi.mocked(clusterService.getClustersConfig).mockReturnValueOnce([
			clusterAlpha,
			clusterBeta
		] as any)
		vi.mocked(clusterService.resolveClusterTargets).mockReturnValueOnce([clusterAlpha as any])

		vi.mocked(h3.readBody).mockResolvedValueOnce({
			username: 'alice',
			display_name: 'Alice Updated',
			adm_status: 1
		})

		vi.mocked(clusterService.forwardToCluster).mockResolvedValueOnce({
			status: 200,
			headers: new Headers(),
			data: {
				data: {
					user_id: 'alpha-1',
					username: 'alice',
					display_name: 'Alice Updated',
					adm_status: 1
				}
			}
		})

		const event = {
			context: {
				params: { userId: 'alpha-1' }
			}
		} as any

		const res = await patchUserHandler(event)

		expect(clusterService.forwardToCluster).toHaveBeenCalledTimes(1)
		expect(clusterService.forwardToCluster).toHaveBeenCalledWith(
			clusterAlpha,
			'/api/users/alpha-1',
			expect.objectContaining({
				method: 'PATCH',
				body: expect.objectContaining({
					display_name: 'Alice Updated'
				})
			})
		)

		expect(res.status_code).toBe(200)
		expect(res.success).toBe(true)
		expect(res.data.cluster_name).toBe('Cluster Alpha')
		expect(res.data.cluster_url).toBe('/alpha')
	})

	it('should return 502 if no target cluster resolved', async () => {
		vi.mocked(clusterService.getClustersConfig).mockReturnValueOnce([])
		vi.mocked(clusterService.resolveClusterTargets).mockReturnValueOnce([])

		const event = {
			context: {
				params: { userId: 'alpha-1' }
			}
		} as any

		const res = await patchUserHandler(event)

		expect(res.status_code).toBe(502)
		expect(res.success).toBe(false)
	})

	it('should handle multi-cluster fallback when multiple clusters targeted', async () => {
		vi.mocked(clusterService.getClustersConfig).mockReturnValueOnce([
			clusterAlpha,
			clusterBeta
		] as any)
		vi.mocked(clusterService.resolveClusterTargets).mockReturnValueOnce([
			clusterAlpha,
			clusterBeta
		] as any)

		vi.mocked(h3.readBody).mockResolvedValueOnce({
			display_name: 'Alice Multi'
		})

		vi.mocked(clusterService.forwardToCluster)
			.mockResolvedValueOnce({
				status: 200,
				headers: new Headers(),
				data: { data: { user_id: 'alpha-1', display_name: 'Alice Multi' } }
			})
			.mockResolvedValueOnce({
				status: 200,
				headers: new Headers(),
				data: { data: { user_id: 'alpha-1', display_name: 'Alice Multi' } }
			})

		const event = {
			context: {
				params: { userId: 'alpha-1' }
			}
		} as any

		const res = await patchUserHandler(event)

		expect(clusterService.forwardToCluster).toHaveBeenCalledTimes(2)
		expect(res.status_code).toBe(200)
		expect(res.success).toBe(true)
		expect(res.data.clusters).toEqual(['Cluster Alpha', 'Cluster Beta'])
	})
})

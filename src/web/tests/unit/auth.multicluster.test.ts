// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { beforeEach, describe, expect, it, vi } from 'vitest'
import * as clusterUtils from '@@/server/services/cluster.service'

const ParsedBodySymbol = Symbol.for('h3ParsedBody')

function createMockEvent(
	path: string,
	body?: any,
	cookies?: string,
	headers: Record<string, any> = {}
) {
	const headersMap = new Map<string, any>()
	const parsedBody = body !== undefined ? body : {}
	const reqHeaders: Record<string, any> = {
		...(cookies ? { cookie: cookies } : {}),
		...headers
	}
	const req: any = {
		headers: reqHeaders,
		[ParsedBodySymbol]: parsedBody,
		body: parsedBody
	}
	return {
		method: 'POST',
		context: {
			params: {
				path
			}
		},
		_body: parsedBody,
		node: {
			req,
			res: {
				getHeader: (key: string) => headersMap.get(key.toLowerCase()),
				setHeader: (key: string, val: any) => {
					headersMap.set(key.toLowerCase(), val)
				},
				removeHeader: (key: string) => {
					headersMap.delete(key.toLowerCase())
				},
				hasHeader: (key: string) => headersMap.has(key.toLowerCase()),
				appendHeader: (key: string, val: any) => {
					const k = key.toLowerCase()
					const existing = headersMap.get(k)
					if (existing) {
						if (Array.isArray(existing)) {
							headersMap.set(k, [...existing, val])
						} else {
							headersMap.set(k, [existing, val])
						}
					} else {
						headersMap.set(k, val)
					}
				}
			}
		}
	}
}

import type { ICluster } from '@@/shared/types/cluster.type'

describe('Auth Route - Multi-Cluster Login & Logout', () => {
	beforeEach(() => {
		vi.restoreAllMocks()
	})

	it('should broadcast login request to all configured clusters when cluster header is not specified', async () => {
		const clusters: ICluster[] = [
			{
				key: 'patrolli1',
				name: 'patrolli1',
				url: 'http://localhost:8080',
				clientPath: '/patrolli1'
			},
			{
				key: 'patrolli2',
				name: 'patrolli2',
				url: 'http://localhost:30001',
				clientPath: '/patrolli2'
			}
		]

		vi.spyOn(clusterUtils, 'getClustersConfig').mockReturnValue(clusters)
		const forwardSpy = vi
			.spyOn(clusterUtils, 'forwardToCluster')
			.mockImplementation(async (cluster) => {
				const headers = new Headers()
				headers.set(
					'set-cookie',
					`patrolli_${cluster.name}=session_${cluster.name}; Path=/`
				)
				return {
					status: 200,
					headers,
					data: {
						status_code: 200,
						success: true,
						data: { message: `Login successful on ${cluster.name}` }
					}
				}
			})

		const { default: authEventHandler } = await import('@@/server/routes/auth/[...path]')

		const mockEvent = createMockEvent('login', { username: 'admin', password: 'password' })

		// Inject mock readBody if h3 uses it
		vi.stubGlobal(
			'readBody',
			vi.fn().mockResolvedValue({ username: 'admin', password: 'password' })
		)

		const result = await authEventHandler(mockEvent as any)

		expect(forwardSpy).toHaveBeenCalledTimes(2)
		expect(forwardSpy).toHaveBeenCalledWith(
			clusters[0],
			'/auth/login',
			expect.objectContaining({ method: 'POST' })
		)
		expect(forwardSpy).toHaveBeenCalledWith(
			clusters[1],
			'/auth/login',
			expect.objectContaining({ method: 'POST' })
		)

		expect(result).toMatchObject({
			status_code: 200,
			success: true
		})
	})

	it('should broadcast logout request to all clusters and clear cookies', async () => {
		const clusters: ICluster[] = [
			{
				key: 'patrolli1',
				name: 'patrolli1',
				url: 'http://localhost:8080',
				clientPath: '/patrolli1'
			},
			{
				key: 'patrolli2',
				name: 'patrolli2',
				url: 'http://localhost:30001',
				clientPath: '/patrolli2'
			}
		]

		vi.spyOn(clusterUtils, 'getClustersConfig').mockReturnValue(clusters)
		const forwardSpy = vi
			.spyOn(clusterUtils, 'forwardToCluster')
			.mockImplementation(async () => {
				return {
					status: 200,
					headers: new Headers(),
					data: { status_code: 200, success: true }
				}
			})

		const { default: authEventHandler } = await import('@@/server/routes/auth/[...path]')

		const mockEvent = createMockEvent('logout', undefined, 'patrolli_session=123')

		const result = await authEventHandler(mockEvent as any)

		expect(forwardSpy).toHaveBeenCalledTimes(2)
		expect(result).toMatchObject({
			status_code: 200,
			success: true
		})
	})

	it('should return 200 and clear cookies on logout even if upstream clusters return errors', async () => {
		const clusters: ICluster[] = [
			{
				key: 'patrolli1',
				name: 'patrolli1',
				url: 'http://localhost:8080',
				clientPath: '/patrolli1'
			}
		]

		vi.spyOn(clusterUtils, 'getClustersConfig').mockReturnValue(clusters)
		vi.spyOn(clusterUtils, 'forwardToCluster').mockResolvedValue({
			status: 401,
			headers: new Headers(),
			data: { status_code: 401, error: { title: 'Unauthorized' } }
		})

		const { default: authEventHandler } = await import('@@/server/routes/auth/[...path]')
		const mockEvent = createMockEvent('logout', undefined, 'patrolli_session=123')

		const result = await authEventHandler(mockEvent as any)

		expect(result).toMatchObject({
			status_code: 200,
			success: true
		})
	})

	it('should target single cluster when cluster header is specified during login', async () => {
		const clusters: ICluster[] = [
			{
				key: 'patrolli1',
				name: 'patrolli1',
				url: 'http://localhost:8080',
				clientPath: '/patrolli1'
			},
			{
				key: 'patrolli2',
				name: 'patrolli2',
				url: 'http://localhost:30001',
				clientPath: '/patrolli2'
			}
		]

		vi.spyOn(clusterUtils, 'getClustersConfig').mockReturnValue(clusters)
		const forwardSpy = vi
			.spyOn(clusterUtils, 'forwardToCluster')
			.mockImplementation(async (cluster) => {
				const headers = new Headers()
				headers.set(
					'set-cookie',
					`patrolli_${cluster.name}=session_${cluster.name}; Path=/`
				)
				return {
					status: 200,
					headers,
					data: {
						status_code: 200,
						success: true,
						data: { message: `Login successful on ${cluster.name}` }
					}
				}
			})

		const { default: authEventHandler } = await import('@@/server/routes/auth/[...path]')

		const mockEvent = createMockEvent('login', { username: 'admin', password: 'password' })
		mockEvent.node.req.headers = { 'x-cluster-url': 'patrolli2' }

		vi.stubGlobal(
			'readBody',
			vi.fn().mockResolvedValue({ username: 'admin', password: 'password' })
		)

		const result = await authEventHandler(mockEvent as any)

		expect(forwardSpy).toHaveBeenCalledTimes(1)
		expect(forwardSpy).toHaveBeenCalledWith(
			clusters[1],
			'/auth/login',
			expect.objectContaining({ method: 'POST' })
		)

		expect(result).toMatchObject({
			status_code: 200,
			success: true
		})
	})
})

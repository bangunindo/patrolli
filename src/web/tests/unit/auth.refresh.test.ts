// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { beforeEach, describe, expect, it, vi } from 'vitest'
import * as clusterUtils from '@@/server/services/cluster.service'
import * as apiBaseService from '@@/server/services/apibase.service'
import type { ICluster } from '@@/shared/types/cluster.type'

const ParsedBodySymbol = Symbol.for('h3ParsedBody')

function createMockEvent(body?: any, cookies?: string, headers: Record<string, any> = {}) {
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
		context: {},
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

describe('Nitro Auth Refresh Endpoint (/api/auth/refresh)', () => {
	beforeEach(() => {
		vi.restoreAllMocks()
	})

	it('should return 401 when no refresh token exists in cookies or body', async () => {
		const { default: refreshHandler } = await import('@@/server/api/auth/refresh.post')
		const mockEvent = createMockEvent(undefined, undefined)

		vi.stubGlobal('readBody', vi.fn().mockResolvedValue({}))

		const result = await refreshHandler(mockEvent as any)

		expect(result).toMatchObject({
			status_code: 401,
			success: false
		})
	})

	it('should refresh token for local provider using cookies', async () => {
		const clusters: ICluster[] = [
			{
				key: 'patrolli1',
				name: 'patrolli1',
				url: 'http://localhost:8080',
				clientPath: '/patrolli1'
			}
		]

		vi.spyOn(clusterUtils, 'getClustersConfig').mockReturnValue(clusters)
		const forwardSpy = vi.spyOn(clusterUtils, 'forwardToCluster').mockResolvedValue({
			status: 200,
			headers: new Headers(),
			data: {
				status_code: 200,
				success: true,
				data: {
					access_token: 'new-local-access-token',
					refresh_token: 'new-local-refresh-token'
				}
			}
		})

		const { default: refreshHandler } = await import('@@/server/api/auth/refresh.post')
		const mockEvent = createMockEvent(
			undefined,
			'refresh_token=old-refresh-token; auth_provider=local'
		)

		vi.stubGlobal('readBody', vi.fn().mockResolvedValue({}))

		const result = await refreshHandler(mockEvent as any)

		expect(forwardSpy).toHaveBeenCalledWith(
			clusters[0],
			'/auth/refresh',
			expect.objectContaining({
				method: 'POST',
				body: { refresh_token: 'old-refresh-token' }
			})
		)

		expect(result).toMatchObject({
			status_code: 200,
			success: true,
			data: {
				access_token: 'new-local-access-token',
				refresh_token: 'new-local-refresh-token'
			}
		})
	})

	it('should refresh token for keycloak provider using cookies', async () => {
		vi.spyOn(clusterUtils, 'getKeycloakConfig').mockReturnValue({
			host: 'http://keycloak.test/realms/patrolli',
			client_id: 'patrolli-test',
			client_secret: 'secret123',
			redirect_uri: 'http://localhost:3000/auth/oidc/callback'
		})

		vi.spyOn(apiBaseService, 'executeHttpRequest').mockResolvedValue({
			status: 200,
			statusText: 'OK',
			headers: {},
			config: {} as any,
			data: {
				access_token: 'new-kc-access-token',
				refresh_token: 'new-kc-refresh-token',
				id_token: 'new-kc-id-token'
			}
		})

		const { default: refreshHandler } = await import('@@/server/api/auth/refresh.post')
		const mockEvent = createMockEvent(
			undefined,
			'refresh_token=old-kc-refresh-token; auth_provider=keycloak'
		)

		vi.stubGlobal('readBody', vi.fn().mockResolvedValue({}))

		const result = await refreshHandler(mockEvent as any)

		expect(result).toMatchObject({
			status_code: 200,
			success: true,
			data: {
				access_token: 'new-kc-access-token',
				refresh_token: 'new-kc-refresh-token'
			}
		})
	})
})

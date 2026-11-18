// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { beforeEach, describe, expect, it, vi } from 'vitest'
import * as clusterService from '@@/server/services/cluster.service'
import * as keycloakApi from '@@/server/modules/keycloak/keycloak.api'
import * as clusterAccessService from '@@/server/services/cluster-access.service'

function createMockEvent(body: Record<string, any> = {}) {
	const setCookies: Record<string, string> = {}
	return {
		node: {
			req: {
				headers: { host: 'localhost:3000' } as Record<string, any>,
				method: 'POST',
				url: '/api/auth/keycloak/callback',
				body
			},
			res: {
				statusCode: 200,
				headers: {} as Record<string, any>,
				setHeader: vi.fn((key: string, val: any) => {
					if (key.toLowerCase() === 'set-cookie') {
						const arr = Array.isArray(val) ? val : [val]
						arr.forEach((c: string) => {
							const [nameVal] = c.split(';')
							const [k, v] = nameVal.split('=')
							if (k) setCookies[k] = decodeURIComponent(v || '')
						})
					}
				}),
				getHeader: vi.fn(),
				removeHeader: vi.fn()
			}
		},
		context: {},
		_body: body,
		_setCookies: setCookies
	}
}

vi.mock('h3', async (importOriginal) => {
	const actual = await importOriginal<typeof import('h3')>()
	return {
		...actual,
		readBody: vi.fn(async (event: any) => event._body || {})
	}
})

describe('Keycloak Auth Controller', () => {
	const mockKcConfig = {
		host: 'https://kc.example.com',
		realm: 'patrolli',
		client_id: 'patrolli-web',
		redirect_uri: 'http://localhost:3000/auth/oidc/callback'
	}

	const jwtPayload = Buffer.from(
		JSON.stringify({
			preferred_username: 'kc-alice',
			sub: 'sub-123',
			email: 'alice@example.com'
		})
	).toString('base64url')
	const mockAccessToken = `hdr.${jwtPayload}.sig`

	beforeEach(() => {
		vi.restoreAllMocks()
		vi.spyOn(clusterService, 'getKeycloakConfig').mockReturnValue(mockKcConfig as any)
		vi.spyOn(clusterAccessService, 'verifyUserClusterAccess').mockResolvedValue(
			undefined as any
		)
	})

	it('handleKeycloakCallback sets HttpOnly cookies and strips raw tokens from response data', async () => {
		vi.spyOn(keycloakApi, 'exchangeKeycloakCode').mockResolvedValueOnce({
			status: 200,
			data: {
				access_token: mockAccessToken,
				refresh_token: 'kc-rt-secret',
				id_token: 'kc-id-secret',
				expires_in: 300
			}
		} as any)

		const { handleKeycloakCallback } =
			await import('@@/server/modules/keycloak/keycloak.controller')
		const mockEvent = createMockEvent({ code: 'auth-code-1' })

		const result = (await handleKeycloakCallback(mockEvent as any)) as any

		expect(result.success).toBe(true)
		expect(result.data).toEqual({
			expires_in: 300,
			user: {
				preferred_username: 'kc-alice',
				username: 'kc-alice',
				sub: 'sub-123',
				email: 'alice@example.com'
			}
		})
		expect(result.data.access_token).toBeUndefined()
		expect(result.data.refresh_token).toBeUndefined()
		expect(result.data.id_token).toBeUndefined()
		expect(mockEvent._setCookies.access_token).toBe(mockAccessToken)
		expect(mockEvent._setCookies.refresh_token).toBe('kc-rt-secret')
	})

	it('handleKeycloakRefresh sets HttpOnly cookies and strips raw tokens from response data', async () => {
		vi.spyOn(keycloakApi, 'refreshKeycloakToken').mockResolvedValueOnce({
			status: 200,
			data: {
				access_token: mockAccessToken,
				refresh_token: 'kc-rt-rotated',
				id_token: 'kc-id-rotated',
				expires_in: 600
			}
		} as any)

		const { handleKeycloakRefresh } =
			await import('@@/server/modules/keycloak/keycloak.controller')
		const mockEvent = createMockEvent({ refresh_token: 'kc-rt-old' })

		const result = (await handleKeycloakRefresh(mockEvent as any)) as any

		expect(result.success).toBe(true)
		expect(result.data).toEqual({
			expires_in: 600,
			user: {
				preferred_username: 'kc-alice',
				username: 'kc-alice',
				sub: 'sub-123',
				email: 'alice@example.com'
			}
		})
		expect(result.data.access_token).toBeUndefined()
		expect(result.data.refresh_token).toBeUndefined()
		expect(result.data.id_token).toBeUndefined()
		expect(mockEvent._setCookies.access_token).toBe(mockAccessToken)
		expect(mockEvent._setCookies.refresh_token).toBe('kc-rt-rotated')
	})

	it('handleKeycloakRefresh falls back to refresh_token HttpOnly cookie when request body is empty', async () => {
		vi.spyOn(keycloakApi, 'refreshKeycloakToken').mockResolvedValueOnce({
			status: 200,
			data: {
				access_token: mockAccessToken,
				refresh_token: 'kc-rt-from-cookie',
				expires_in: 600
			}
		} as any)

		const { handleKeycloakRefresh } =
			await import('@@/server/modules/keycloak/keycloak.controller')
		const mockEvent = createMockEvent({})
		mockEvent.node.req.headers.cookie = 'refresh_token=cookie-rt-123; auth_provider=keycloak'

		const result = (await handleKeycloakRefresh(mockEvent as any)) as any

		expect(keycloakApi.refreshKeycloakToken).toHaveBeenCalledWith(mockKcConfig, 'cookie-rt-123')
		expect(result.success).toBe(true)
		expect(mockEvent._setCookies.access_token).toBe(mockAccessToken)
		expect(mockEvent._setCookies.refresh_token).toBe('kc-rt-from-cookie')
	})

	it('handleKeycloakRefresh returns 401 and clears auth cookies when refresh_token is missing from both body and cookies', async () => {
		const { handleKeycloakRefresh } =
			await import('@@/server/modules/keycloak/keycloak.controller')
		const mockEvent = createMockEvent({})
		mockEvent.node.req.headers.cookie = 'is_authenticated=true; auth_provider=keycloak'

		const result = (await handleKeycloakRefresh(mockEvent as any)) as any

		expect(mockEvent.node.res.statusCode).toBe(401)
		expect(result).toMatchObject({
			status_code: 401,
			success: false,
			error: {
				title: 'Unauthorized'
			}
		})
		expect(mockEvent._setCookies.is_authenticated).toBe('')
	})

	it('handleOidcCallback sets HttpOnly cookies, verifies cluster access, and renders OIDC success HTML without raw tokens', async () => {
		vi.spyOn(keycloakApi, 'exchangeKeycloakCode').mockResolvedValueOnce({
			status: 200,
			data: {
				access_token: mockAccessToken,
				refresh_token: 'kc-rt-oidc',
				id_token: 'kc-id-oidc',
				expires_in: 300
			}
		} as any)

		const { handleOidcCallback } = await import('@@/server/modules/auth/auth.controller')
		const mockEvent = createMockEvent() as any
		mockEvent.path = '/auth/oidc/callback?code=oidc-code-123'
		mockEvent.node.req.method = 'GET'
		mockEvent.node.req.url = '/auth/oidc/callback?code=oidc-code-123'

		const html = await handleOidcCallback(mockEvent)

		expect(clusterAccessService.verifyUserClusterAccess).toHaveBeenCalledWith(
			mockEvent,
			mockAccessToken
		)
		expect(mockEvent._setCookies.access_token).toBe(mockAccessToken)
		expect(mockEvent._setCookies.refresh_token).toBe('kc-rt-oidc')
		expect(html).toContain("window.location.replace('/')")
		expect(html).not.toContain(mockAccessToken)
		expect(html).not.toContain('kc-rt-oidc')
	})
})

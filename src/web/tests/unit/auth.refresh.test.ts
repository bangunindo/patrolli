// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { beforeEach, describe, expect, it, vi } from 'vitest'
import * as clusterUtils from '@@/server/services/cluster.service'
import * as apiBaseService from '@@/server/services/apibase.service'
import * as oidcService from '@@/server/modules/oidc/oidc.service'
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
			success: false,
			is_session_invalid: true
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
			'/api/v1/auth/refresh',
			expect.objectContaining({
				method: 'POST',
				body: { refresh_token: 'old-refresh-token' }
			})
		)

		expect(result).toMatchObject({
			status_code: 200,
			success: true
		})
		expect((result as any).data?.access_token).toBeUndefined()
		expect((result as any).data?.refresh_token).toBeUndefined()

		const setCookies = mockEvent.node.res.getHeader('set-cookie')
		const cookieStr = Array.isArray(setCookies) ? setCookies.join('\n') : String(setCookies)
		expect(cookieStr).toContain('access_token=new-local-access-token')
		expect(cookieStr).toContain('refresh_token=new-local-refresh-token')
		expect(cookieStr).toContain('active_cluster_url=%2Fpatrolli1')
		expect(cookieStr).toContain('HttpOnly')
	})

	it('should refresh token for oidc provider using cookies', async () => {
		vi.spyOn(clusterUtils, 'getOidcConfig').mockReturnValue({
			issuer: 'http://idp.test/realms/patrolli',
			client_id: 'patrolli-test',
			client_secret: 'secret123',
			redirect_uri: 'http://localhost:3000/auth/oidc/callback',
			scopes: 'openid profile email',
			token_source: 'access_token',
			display_name: 'SSO'
		})

		vi.spyOn(oidcService, 'refreshOidcTokens').mockResolvedValue({
			access_token: 'new-oidc-access-token',
			refresh_token: 'new-oidc-refresh-token',
			id_token: 'new-oidc-id-token'
		})

		const { default: refreshHandler } = await import('@@/server/api/auth/refresh.post')
		const mockEvent = createMockEvent(
			undefined,
			'refresh_token=old-oidc-refresh-token; auth_provider=oidc'
		)

		vi.stubGlobal('readBody', vi.fn().mockResolvedValue({}))

		const result = await refreshHandler(mockEvent as any)

		expect(result).toMatchObject({
			status_code: 200,
			success: true
		})
		expect((result as any).data?.access_token).toBeUndefined()
		expect((result as any).data?.refresh_token).toBeUndefined()

		const setCookies = mockEvent.node.res.getHeader('set-cookie')
		const cookieStr = Array.isArray(setCookies) ? setCookies.join('\n') : String(setCookies)
		expect(cookieStr).toContain('access_token=new-oidc-access-token')
		expect(cookieStr).toContain('refresh_token=new-oidc-refresh-token')
		expect(cookieStr).toContain('HttpOnly')
	})

	it('should refresh token for legacy auth_provider=keycloak cookie and via /api/auth/keycloak/refresh', async () => {
		vi.spyOn(clusterUtils, 'getOidcConfig').mockReturnValue({
			issuer: 'http://idp.test/realms/patrolli',
			client_id: 'patrolli-test',
			client_secret: 'secret123',
			redirect_uri: 'http://localhost:3000/auth/oidc/callback',
			scopes: 'openid profile email',
			token_source: 'access_token',
			display_name: 'Keycloak'
		})

		const refreshSpy = vi.spyOn(oidcService, 'refreshOidcTokens').mockResolvedValue({
			access_token: 'migrated-oidc-access-token',
			refresh_token: 'migrated-oidc-refresh-token',
			id_token: 'migrated-oidc-id-token'
		})

		const { default: refreshHandler } = await import('@@/server/api/auth/refresh.post')
		const legacyCookieEvent = createMockEvent(
			undefined,
			'refresh_token=legacy-kc-rt; auth_provider=keycloak'
		)
		vi.stubGlobal('readBody', vi.fn().mockResolvedValue({}))

		const result1 = await refreshHandler(legacyCookieEvent as any)
		expect(result1).toMatchObject({ status_code: 200, success: true })
		const cookies1 = legacyCookieEvent.node.res.getHeader('set-cookie')
		const cookieStr1 = Array.isArray(cookies1) ? cookies1.join('\n') : String(cookies1)
		expect(cookieStr1).toContain('access_token=migrated-oidc-access-token')
		expect(cookieStr1).toContain('auth_provider=oidc')

		const { default: kcRefreshHandler } =
			await import('@@/server/api/auth/keycloak/refresh.post')
		const kcEndpointEvent = createMockEvent({ refresh_token: 'direct-kc-rt' })
		const result2 = await kcRefreshHandler(kcEndpointEvent as any)
		expect(result2).toMatchObject({ status_code: 200, success: true })
		expect(refreshSpy).toHaveBeenCalledTimes(2)
	})

	it('should execute refresh statelessly without in-memory map and set access_token_exp cookie', async () => {
		vi.spyOn(clusterUtils, 'getOidcConfig').mockReturnValue({
			issuer: 'http://idp.test/realms/patrolli',
			client_id: 'patrolli-test',
			client_secret: 'secret123',
			redirect_uri: 'http://localhost:3000/auth/oidc/callback',
			scopes: 'openid profile email',
			token_source: 'access_token',
			display_name: 'SSO'
		})

		const expSecs = Math.floor(Date.now() / 1000) + 300
		const payloadB64 = Buffer.from(JSON.stringify({ exp: expSecs })).toString('base64url')
		const jwtWithExp = `hdr.${payloadB64}.sig`

		const refreshSpy = vi
			.spyOn(oidcService, 'refreshOidcTokens')
			.mockImplementation(async () => ({
				access_token: jwtWithExp,
				refresh_token: 'stateless-rt'
			}))

		const { default: refreshHandler } = await import('@@/server/api/auth/refresh.post')
		vi.stubGlobal('readBody', vi.fn().mockResolvedValue({}))

		const ev1 = createMockEvent(undefined, 'refresh_token=stateless-rt-1; auth_provider=oidc')
		const ev2 = createMockEvent(undefined, 'refresh_token=stateless-rt-1; auth_provider=oidc')

		const [r1, r2] = await Promise.all([refreshHandler(ev1 as any), refreshHandler(ev2 as any)])

		expect(refreshSpy).toHaveBeenCalledTimes(2)
		expect(r1).toMatchObject({ status_code: 200, success: true })
		expect(r2).toMatchObject({ status_code: 200, success: true })
		const cookieStr = Array.isArray(ev1.node.res.getHeader('set-cookie'))
			? (ev1.node.res.getHeader('set-cookie') as string[]).join('\n')
			: String(ev1.node.res.getHeader('set-cookie'))
		expect(cookieStr).toContain(`access_token=${jwtWithExp}`)
		expect(cookieStr).toContain(`access_token_exp=${expSecs * 1000}`)
	})

	it('should NOT clear auth cookies when upstream refresh returns 502', async () => {
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
			status: 502,
			headers: new Headers(),
			data: { status_code: 502, success: false }
		})

		const { default: refreshHandler } = await import('@@/server/api/auth/refresh.post')
		const mockEvent = createMockEvent(undefined, 'refresh_token=keep-rt; auth_provider=local')
		vi.stubGlobal('readBody', vi.fn().mockResolvedValue({}))

		const result = await refreshHandler(mockEvent as any)
		expect(result).toMatchObject({ status_code: 502, success: false })
		const setCookies = mockEvent.node.res.getHeader('set-cookie')
		expect(setCookies).toBeUndefined()
	})

	it('should transparently refresh and retry GET proxy request on 401', async () => {
		const clusters: ICluster[] = [
			{
				key: 'patrolli1',
				name: 'patrolli1',
				url: 'http://localhost:8080',
				clientPath: '/patrolli1'
			}
		]
		vi.spyOn(clusterUtils, 'getClustersConfig').mockReturnValue(clusters)
		const forwardSpy = vi
			.spyOn(clusterUtils, 'forwardToCluster')
			.mockResolvedValueOnce({
				status: 401,
				headers: new Headers(),
				data: { status_code: 401, success: false }
			})
			.mockResolvedValueOnce({
				status: 200,
				headers: new Headers(),
				data: {
					status_code: 200,
					success: true,
					data: {
						access_token: 'refreshed-proxy-at',
						refresh_token: 'refreshed-proxy-rt'
					}
				}
			})
			.mockResolvedValueOnce({
				status: 200,
				headers: new Headers(),
				data: [{ id: 1 }]
			})

		const { handleProxyGet } = await import('@@/server/modules/proxy/proxy.controller')
		const mockEvent = createMockEvent(
			undefined,
			'access_token=expired-at; refresh_token=valid-rt; auth_provider=local'
		)
		mockEvent.method = 'GET'
		mockEvent.context = { params: { path: 'hosts' } }

		const result = await handleProxyGet(mockEvent as any)
		expect(forwardSpy).toHaveBeenCalledTimes(3)
		expect(result).toMatchObject({ status_code: 200, success: true, data: [{ id: 1 }] })
		expect(String(mockEvent.node.res.getHeader('set-cookie'))).toContain('refreshed-proxy-at')
	})

	it('should mark session invalid and clear cookies on OIDC 400 invalid_grant', async () => {
		vi.spyOn(clusterUtils, 'getOidcConfig').mockReturnValue({
			issuer: 'http://idp.test/realms/patrolli',
			client_id: 'patrolli-test',
			client_secret: 'secret123',
			redirect_uri: 'http://localhost:3000/auth/oidc/callback',
			scopes: 'openid profile email',
			token_source: 'access_token',
			display_name: 'SSO'
		})

		const oauthErr: any = new Error('Session not active')
		oauthErr.code = 'OAUTH_RESPONSE_BODY_ERROR'
		oauthErr.status = 400
		oauthErr.error = 'invalid_grant'
		oauthErr.error_description = 'Session not active'
		vi.spyOn(oidcService, 'refreshOidcTokens').mockRejectedValue(oauthErr)

		const { default: refreshHandler } = await import('@@/server/api/auth/refresh.post')
		const mockEvent = createMockEvent(
			undefined,
			'refresh_token=expired-oidc-rt; auth_provider=oidc'
		)
		vi.stubGlobal('readBody', vi.fn().mockResolvedValue({}))

		const result = await refreshHandler(mockEvent as any)
		expect(result).toMatchObject({
			status_code: 400,
			success: false,
			is_session_invalid: true
		})
		const setCookies = mockEvent.node.res.getHeader('set-cookie')
		const cookieStr = Array.isArray(setCookies) ? setCookies.join('\n') : String(setCookies)
		expect(cookieStr).toContain('refresh_token=;')
	})

	it('should ignore non-error HTTP status codes on OIDC refresh exceptions and fall back to 502', async () => {
		vi.spyOn(clusterUtils, 'getOidcConfig').mockReturnValue({
			issuer: 'http://idp.test/realms/patrolli',
			client_id: 'patrolli-test',
			client_secret: 'secret123',
			redirect_uri: 'http://localhost:3000/auth/oidc/callback',
			scopes: 'openid profile email',
			token_source: 'access_token',
			display_name: 'SSO'
		})

		const unexpectedErr: any = new Error('Malformed JSON on 200 OK')
		unexpectedErr.status = 200
		vi.spyOn(oidcService, 'refreshOidcTokens').mockRejectedValue(unexpectedErr)

		const { default: refreshHandler } = await import('@@/server/api/auth/refresh.post')
		const mockEvent = createMockEvent(undefined, 'refresh_token=oidc-rt; auth_provider=oidc')
		vi.stubGlobal('readBody', vi.fn().mockResolvedValue({}))

		const result = await refreshHandler(mockEvent as any)
		expect(result).toMatchObject({
			status_code: 502,
			success: false,
			is_session_invalid: false
		})
	})

	it('should mark session invalid (401) when token_source is id_token and refresh returns opaque access_token without id_token', async () => {
		vi.spyOn(clusterUtils, 'getOidcConfig').mockReturnValue({
			issuer: 'http://idp.test/realms/patrolli',
			client_id: 'patrolli-test',
			client_secret: 'secret123',
			redirect_uri: 'http://localhost:3000/auth/oidc/callback',
			scopes: 'openid profile email',
			token_source: 'id_token',
			display_name: 'SSO'
		})

		vi.spyOn(oidcService, 'refreshOidcTokens').mockResolvedValue({
			access_token: 'opaque-non-jwt-token',
			refresh_token: 'new-rt'
		})

		const { default: refreshHandler } = await import('@@/server/api/auth/refresh.post')
		const mockEvent = createMockEvent(undefined, 'refresh_token=oidc-rt; auth_provider=oidc')
		vi.stubGlobal('readBody', vi.fn().mockResolvedValue({}))

		const result = await refreshHandler(mockEvent as any)
		expect(result).toMatchObject({
			status_code: 401,
			success: false,
			is_session_invalid: true,
			error: {
				title: 'OIDC Refresh Error',
				detail: expect.stringContaining('id_token')
			}
		})
	})

	it('should forward Authorization Bearer without forwarding Cookie header to upstream cluster', async () => {
		const execSpy = vi.spyOn(apiBaseService, 'executeHttpRequest').mockResolvedValue({
			status: 200,
			statusText: 'OK',
			headers: {},
			config: {} as any,
			data: { ok: true }
		})

		await clusterUtils.forwardToCluster(
			{
				key: 'patrolli1',
				name: 'patrolli1',
				url: 'http://localhost:8080',
				clientPath: '/patrolli1'
			},
			'/api/v1/hosts',
			{
				method: 'GET',
				headers: {
					cookie: 'access_token=jwt-at; refresh_token=jwt-rt; id_token=jwt-id'
				}
			}
		)

		expect(execSpy).toHaveBeenCalledTimes(1)
		const forwardedHeaders = (execSpy.mock.calls[0]?.[0] as any)?.headers
		expect(forwardedHeaders?.authorization).toBe('Bearer jwt-at')
		expect(forwardedHeaders?.cookie).toBeUndefined()
	})

	it('should pre-emptively refresh expired access_token before calling upstream cluster in handleProxyGet', async () => {
		const clusters: ICluster[] = [
			{
				key: 'patrolli1',
				name: 'patrolli1',
				url: 'http://localhost:8080',
				clientPath: '/patrolli1'
			}
		]
		vi.spyOn(clusterUtils, 'getClustersConfig').mockReturnValue(clusters)

		const expiredPayload = Buffer.from(
			JSON.stringify({ exp: Math.floor(Date.now() / 1000) - 60 })
		).toString('base64url')
		const expiredJwt = `hdr.${expiredPayload}.sig`

		const forwardSpy = vi
			.spyOn(clusterUtils, 'forwardToCluster')
			.mockResolvedValueOnce({
				status: 200,
				headers: new Headers(),
				data: {
					status_code: 200,
					success: true,
					data: {
						access_token: 'proactive-at',
						refresh_token: 'proactive-rt'
					}
				}
			})
			.mockResolvedValueOnce({
				status: 200,
				headers: new Headers(),
				data: [{ id: 42 }]
			})

		const { handleProxyGet } = await import('@@/server/modules/proxy/proxy.controller')
		const mockEvent = createMockEvent(
			undefined,
			`access_token=${expiredJwt}; refresh_token=valid-rt; auth_provider=local`
		)
		mockEvent.method = 'GET'
		mockEvent.context = { params: { path: 'hosts' } }

		const result = await handleProxyGet(mockEvent as any)
		expect(forwardSpy).toHaveBeenCalledTimes(2)
		expect(forwardSpy.mock.calls[0]?.[1]).toBe('/api/v1/auth/refresh')
		expect(forwardSpy.mock.calls[1]?.[1]).toBe('/api/v1/hosts')
		expect(forwardSpy.mock.calls[1]?.[2]?.headers?.authorization).toBe('Bearer proactive-at')
		expect(result).toMatchObject({ status_code: 200, success: true, data: [{ id: 42 }] })
	})

	it('should route local performTokenRefresh to the target cluster specified by clusterHeader', async () => {
		const clusters: ICluster[] = [
			{
				key: 'cluster-a',
				name: 'cluster-a',
				url: 'http://cluster-a:8080',
				clientPath: '/cluster-a'
			},
			{
				key: 'cluster-b',
				name: 'cluster-b',
				url: 'http://cluster-b:8080',
				clientPath: '/cluster-b'
			}
		]
		vi.spyOn(clusterUtils, 'getClustersConfig').mockReturnValue(clusters)

		const forwardSpy = vi.spyOn(clusterUtils, 'forwardToCluster').mockImplementation(
			async (cluster) =>
				({
					status: 200,
					headers: new Headers(),
					data: {
						access_token: `at-${cluster.key}`,
						refresh_token: `rt-${cluster.key}`
					}
				}) as any
		)

		const { performTokenRefresh } = await import('@@/server/services/auth-refresh.service')
		const eventA = createMockEvent()
		const eventB = createMockEvent()

		const [resA, resB] = await Promise.all([
			performTokenRefresh(eventA as any, {
				refreshToken: 'shared-rt',
				authProvider: 'local',
				clusterHeader: 'cluster-a'
			}),
			performTokenRefresh(eventB as any, {
				refreshToken: 'shared-rt',
				authProvider: 'local',
				clusterHeader: 'cluster-b'
			})
		])

		expect(forwardSpy).toHaveBeenCalledTimes(2)
		expect(resA.tokens?.access_token).toBe('at-cluster-a')
		expect(resB.tokens?.access_token).toBe('at-cluster-b')
	})

	it('preserves non-token payload on handleAuthProxy for endpoints like /auth/change-password', async () => {
		const clusterService = await import('@@/server/services/cluster.service')
		const { handleAuthProxy } = await import('@@/server/modules/auth/auth.controller')
		const clusterA = { name: 'A', url: 'http://a', clientPath: '/a', key: 'a' }
		vi.spyOn(clusterService, 'getClustersConfig').mockReturnValue([clusterA] as any)
		vi.spyOn(clusterService, 'resolveClusterTargets').mockReturnValue([clusterA] as any)

		const expectedBody = {
			status_code: 200,
			success: true,
			data: { message: 'Password updated successfully' }
		}
		vi.spyOn(clusterService, 'forwardToCluster').mockResolvedValue({
			status: 200,
			headers: new Headers(),
			data: expectedBody
		} as any)

		const event = createMockEvent({
			method: 'POST',
			url: '/auth/change-password',
			params: { _: 'change-password' },
			body: { old_password: 'old', new_password: 'new' }
		})

		const res = await handleAuthProxy(event as any)
		expect(res).toEqual(expectedBody)
		const setCookies = event.node.res.getHeader('set-cookie')
		expect(setCookies).toBeUndefined()
	})

	it('should not re-refresh in resolveEffectiveRequestHeaders when authorization or context token is already fresh', async () => {
		const clusters: ICluster[] = [
			{
				key: 'patrolli1',
				name: 'patrolli1',
				url: 'http://localhost:8080',
				clientPath: '/patrolli1'
			}
		]
		vi.spyOn(clusterUtils, 'getClustersConfig').mockReturnValue(clusters)

		const expiredPayload = Buffer.from(
			JSON.stringify({ exp: Math.floor(Date.now() / 1000) - 60 })
		).toString('base64url')
		const expiredJwt = `hdr.${expiredPayload}.sig`

		const freshPayload = Buffer.from(
			JSON.stringify({ exp: Math.floor(Date.now() / 1000) + 300 })
		).toString('base64url')
		const freshJwt = `hdr.${freshPayload}.sig`

		const forwardSpy = vi.spyOn(clusterUtils, 'forwardToCluster').mockResolvedValue({
			status: 200,
			headers: new Headers(),
			data: {
				status_code: 200,
				success: true,
				data: {
					access_token: freshJwt,
					refresh_token: 'new-rt'
				}
			}
		})

		const { resolveEffectiveRequestHeaders } =
			await import('@@/server/services/auth-refresh.service')
		const mockEvent = createMockEvent(
			undefined,
			`access_token=${expiredJwt}; refresh_token=valid-rt; auth_provider=local`
		)

		const firstHeaders = await resolveEffectiveRequestHeaders(mockEvent as any, {})
		expect(firstHeaders.authorization).toBe(`Bearer ${freshJwt}`)
		expect(forwardSpy).toHaveBeenCalledTimes(1)

		// Second call with firstHeaders or same event context must NOT trigger another refresh
		const secondHeaders = await resolveEffectiveRequestHeaders(mockEvent as any, firstHeaders)
		expect(secondHeaders.authorization).toBe(`Bearer ${freshJwt}`)

		const thirdHeaders = await resolveEffectiveRequestHeaders(mockEvent as any, {})
		expect(thirdHeaders.authorization).toBe(`Bearer ${freshJwt}`)
		expect(forwardSpy).toHaveBeenCalledTimes(1)
	})

	it('should support performTokenRefresh without an H3Event', async () => {
		const clusters: ICluster[] = [
			{
				key: 'patrolli1',
				name: 'patrolli1',
				url: 'http://localhost:8080',
				clientPath: '/patrolli1'
			}
		]
		vi.spyOn(clusterUtils, 'getClustersConfig').mockReturnValue(clusters)

		vi.spyOn(clusterUtils, 'forwardToCluster').mockResolvedValueOnce({
			status: 200,
			headers: new Headers(),
			data: {
				status_code: 200,
				success: true,
				data: {
					access_token: 'ws-refreshed-at',
					refresh_token: 'ws-refreshed-rt'
				}
			}
		})

		const { performTokenRefresh } = await import('@@/server/services/auth-refresh.service')
		const result = await performTokenRefresh(undefined, {
			refreshToken: 'old-rt',
			authProvider: 'local'
		})

		expect(result.ok).toBe(true)
		expect(result.tokens?.access_token).toBe('ws-refreshed-at')
	})

	it('should replace PascalCase Authorization header without duplicating keys when refreshing headers', async () => {
		const clusters: ICluster[] = [
			{
				key: 'patrolli1',
				name: 'patrolli1',
				url: 'http://localhost:8080',
				clientPath: '/patrolli1'
			}
		]
		vi.spyOn(clusterUtils, 'getClustersConfig').mockReturnValue(clusters)

		const expiredPayload = Buffer.from(
			JSON.stringify({ exp: Math.floor(Date.now() / 1000) - 60 })
		).toString('base64url')
		const expiredJwt = `hdr.${expiredPayload}.sig`

		const freshPayload = Buffer.from(
			JSON.stringify({ exp: Math.floor(Date.now() / 1000) + 300 })
		).toString('base64url')
		const freshJwt = `hdr.${freshPayload}.sig`

		vi.spyOn(clusterUtils, 'forwardToCluster').mockResolvedValueOnce({
			status: 200,
			headers: new Headers(),
			data: {
				status_code: 200,
				success: true,
				data: {
					access_token: freshJwt,
					refresh_token: 'new-rt'
				}
			}
		})

		const { resolveEffectiveRequestHeaders } =
			await import('@@/server/services/auth-refresh.service')
		const mockEvent = createMockEvent(
			undefined,
			`access_token=${expiredJwt}; refresh_token=valid-rt; auth_provider=local`
		)

		const headers = await resolveEffectiveRequestHeaders(mockEvent as any, {
			Authorization: `Bearer ${expiredJwt}`,
			'x-custom': '1'
		})

		expect(headers.authorization).toBe(`Bearer ${freshJwt}`)
		expect(headers.Authorization).toBeUndefined()
		expect(headers['x-custom']).toBe('1')
	})

	it('should not trigger a second refresh on 401 when proactive refresh already succeeded in the same request', async () => {
		const clusters: ICluster[] = [
			{
				key: 'patrolli1',
				name: 'patrolli1',
				url: 'http://localhost:8080',
				clientPath: '/patrolli1'
			}
		]
		vi.spyOn(clusterUtils, 'getClustersConfig').mockReturnValue(clusters)

		const expiredPayload = Buffer.from(
			JSON.stringify({ exp: Math.floor(Date.now() / 1000) - 60 })
		).toString('base64url')
		const expiredJwt = `hdr.${expiredPayload}.sig`

		const freshPayload = Buffer.from(
			JSON.stringify({ exp: Math.floor(Date.now() / 1000) + 300 })
		).toString('base64url')
		const freshJwt = `hdr.${freshPayload}.sig`

		const forwardSpy = vi.spyOn(clusterUtils, 'forwardToCluster').mockResolvedValueOnce({
			status: 200,
			headers: new Headers(),
			data: {
				status_code: 200,
				success: true,
				data: {
					access_token: freshJwt,
					refresh_token: 'rotated-rt'
				}
			}
		})

		const { executeClusterRequestsWithAuthRetry } =
			await import('@@/server/services/auth-refresh.service')
		const mockEvent = createMockEvent(
			undefined,
			`access_token=${expiredJwt}; refresh_token=old-rt; auth_provider=local`
		)

		const executeSpy = vi.fn().mockResolvedValue({ status: 401 })

		const { results } = await executeClusterRequestsWithAuthRetry(mockEvent as any, {
			items: clusters,
			reqHeaders: {},
			execute: executeSpy
		})

		expect(forwardSpy).toHaveBeenCalledTimes(1)
		expect(executeSpy).toHaveBeenCalledTimes(1)
		expect(results[0]?.status).toBe(401)
	})

	it('should set active_cluster_url cookie on local login via handleAuthProxy', async () => {
		const clusterService = await import('@@/server/services/cluster.service')
		const clusterAccessService = await import('@@/server/services/cluster-access.service')
		const { handleAuthProxy } = await import('@@/server/modules/auth/auth.controller')
		const clusterA = { name: 'A', url: 'http://a', clientPath: '/a', key: 'a' }
		vi.spyOn(clusterService, 'getClustersConfig').mockReturnValue([clusterA] as any)
		vi.spyOn(clusterService, 'resolveClusterTargets').mockReturnValue([clusterA] as any)
		vi.spyOn(clusterAccessService, 'verifyUserClusterAccess').mockResolvedValue([])
		vi.spyOn(clusterService, 'forwardToCluster').mockResolvedValue({
			status: 200,
			headers: new Headers(),
			data: {
				access_token: 'login-at',
				refresh_token: 'login-rt'
			}
		})

		const mockEvent = createMockEvent({ username: 'admin', password: 'pw' }, undefined, {
			'x-cluster-url': '/a'
		})
		mockEvent.method = 'POST'
		mockEvent.context = { params: { path: 'login' } }

		const result = await handleAuthProxy(mockEvent as any)
		expect(result).toMatchObject({ status_code: 200, success: true })

		const setCookies = mockEvent.node.res.getHeader('set-cookie')
		const cookieStr = Array.isArray(setCookies) ? setCookies.join('\n') : String(setCookies)
		expect(cookieStr).toContain('access_token=login-at')
		expect(cookieStr).toContain('active_cluster_url=%2Fa')
	})

	it('initializes event.context safely when performTokenRefresh receives an event without context', async () => {
		const clusters: ICluster[] = [
			{
				key: 'patrolli1',
				name: 'patrolli1',
				url: 'http://localhost:8080',
				clientPath: '/patrolli1'
			}
		]
		vi.spyOn(clusterUtils, 'getClustersConfig').mockReturnValue(clusters)
		vi.spyOn(clusterUtils, 'forwardToCluster').mockResolvedValueOnce({
			status: 200,
			headers: new Headers(),
			data: {
				access_token: 'ctx-init-at',
				refresh_token: 'ctx-init-rt',
				id_token: 'ctx-init-it'
			}
		})

		const { performTokenRefresh } = await import('@@/server/services/auth-refresh.service')
		const mockEvent = createMockEvent() as any
		delete mockEvent.context

		const result = await performTokenRefresh(mockEvent, {
			refreshToken: 'valid-rt',
			authProvider: 'local'
		})

		expect(result.ok).toBe(true)
		expect(mockEvent.context?._refreshedAccessToken).toBe('ctx-init-at')
		expect(mockEvent.context?._refreshedIdToken).toBe('ctx-init-it')
	})

	it('should transparently refresh and retry /auth/change-password on 401 in handleAuthProxy', async () => {
		const clusterService = await import('@@/server/services/cluster.service')
		const { handleAuthProxy } = await import('@@/server/modules/auth/auth.controller')
		const clusterA = { name: 'A', url: 'http://a', clientPath: '/a', key: 'a' }
		vi.spyOn(clusterService, 'getClustersConfig').mockReturnValue([clusterA] as any)
		vi.spyOn(clusterService, 'resolveClusterTargets').mockReturnValue([clusterA] as any)

		const expectedBody = {
			status_code: 200,
			success: true,
			data: { message: 'Password updated successfully' }
		}

		const forwardSpy = vi
			.spyOn(clusterService, 'forwardToCluster')
			.mockResolvedValueOnce({
				status: 401,
				headers: new Headers(),
				data: { status_code: 401, success: false }
			} as any)
			.mockResolvedValueOnce({
				status: 200,
				headers: new Headers(),
				data: {
					access_token: 'refreshed-cp-at',
					refresh_token: 'refreshed-cp-rt'
				}
			} as any)
			.mockResolvedValueOnce({
				status: 200,
				headers: new Headers(),
				data: expectedBody
			} as any)

		const event = createMockEvent(
			{ old_password: 'old', new_password: 'new' },
			'access_token=revoked-at; refresh_token=valid-rt; auth_provider=local'
		)
		event.method = 'POST'
		event.context = { params: { path: 'change-password' } }

		const res = await handleAuthProxy(event as any)
		expect(forwardSpy).toHaveBeenCalledTimes(3)
		expect(forwardSpy.mock.calls[1]?.[1]).toBe('/api/v1/auth/refresh')
		expect(forwardSpy.mock.calls[2]?.[1]).toBe('/api/v1/auth/change-password')
		expect(forwardSpy.mock.calls[2]?.[2]?.headers?.authorization).toBe('Bearer refreshed-cp-at')
		expect(res).toEqual(expectedBody)
	})

	it('should NOT mark session invalid or clear cookies on multi-cluster refresh when one cluster returns 401 and another returns 502', async () => {
		const clusters: ICluster[] = [
			{
				key: 'cluster-a',
				name: 'cluster-a',
				url: 'http://cluster-a:8080',
				clientPath: '/cluster-a'
			},
			{
				key: 'cluster-b',
				name: 'cluster-b',
				url: 'http://cluster-b:8080',
				clientPath: '/cluster-b'
			}
		]
		vi.spyOn(clusterUtils, 'getClustersConfig').mockReturnValue(clusters)
		vi.spyOn(clusterUtils, 'forwardToCluster').mockImplementation(async (cluster) => {
			if (cluster.key === 'cluster-a') {
				return {
					status: 401,
					headers: new Headers(),
					data: { status_code: 401, success: false }
				}
			}
			return {
				status: 502,
				headers: new Headers(),
				data: { status_code: 502, success: false }
			}
		})

		const { default: refreshHandler } = await import('@@/server/api/auth/refresh.post')
		const mockEvent = createMockEvent(undefined, 'refresh_token=keep-rt; auth_provider=local')
		vi.stubGlobal('readBody', vi.fn().mockResolvedValue({}))

		const result = await refreshHandler(mockEvent as any)
		expect(result).toMatchObject({ status_code: 502, success: false })
		const setCookies = mockEvent.node.res.getHeader('set-cookie')
		expect(setCookies).toBeUndefined()
	})

	it('should strip both Authorization and Cookie headers on refresh so stale access_token cookie is not re-attached as Bearer token by forwardToCluster', async () => {
		const clusters: ICluster[] = [
			{
				key: 'cluster-a',
				name: 'cluster-a',
				url: 'http://cluster-a:8080',
				clientPath: '/cluster-a'
			}
		]
		vi.spyOn(clusterUtils, 'getClustersConfig').mockReturnValue(clusters)
		const httpSpy = vi.spyOn(apiBaseService, 'executeHttpRequest').mockResolvedValue({
			status: 200,
			headers: {},
			data: {
				access_token: 'fresh-at-no-cookie-leak',
				refresh_token: 'fresh-rt-no-cookie-leak'
			}
		} as any)

		const { default: refreshHandler } = await import('@@/server/api/auth/refresh.post')
		const mockEvent = createMockEvent(
			undefined,
			'access_token=expired-cookie-jwt; refresh_token=valid-rt-1; auth_provider=local',
			{ authorization: 'Bearer expired-header-jwt' }
		)
		vi.stubGlobal('readBody', vi.fn().mockResolvedValue({}))

		const result = await refreshHandler(mockEvent as any)
		expect(result).toMatchObject({ status_code: 200, success: true })
		expect(httpSpy).toHaveBeenCalledTimes(1)
		const sentHeaders = httpSpy.mock.calls[0]?.[0]?.headers || {}
		expect(sentHeaders.authorization).toBeUndefined()
		expect(sentHeaders.Authorization).toBeUndefined()
		expect(sentHeaders.cookie).toBeUndefined()
	})

	it('should strip stale Authorization and Cookie headers when forwarding /auth/login in handleAuthProxy', async () => {
		const clusters: ICluster[] = [
			{
				key: 'cluster-a',
				name: 'cluster-a',
				url: 'http://cluster-a:8080',
				clientPath: '/cluster-a'
			}
		]
		vi.spyOn(clusterUtils, 'getClustersConfig').mockReturnValue(clusters)
		vi.spyOn(clusterUtils, 'resolveClusterTargets').mockReturnValue(clusters)
		const httpSpy = vi.spyOn(apiBaseService, 'executeHttpRequest').mockResolvedValue({
			status: 200,
			headers: {},
			data: {
				access_token: 'login-fresh-at',
				refresh_token: 'login-fresh-rt'
			}
		} as any)

		const { handleAuthProxy } = await import('@@/server/modules/auth/auth.controller')
		const event = createMockEvent(
			{ username: 'admin', password: 'secret' },
			'access_token=stale-login-cookie-jwt; refresh_token=stale-rt',
			{ authorization: 'Bearer stale-login-header-jwt' }
		)
		event.method = 'POST'
		event.context = { params: { path: 'login' } }

		const res = await handleAuthProxy(event as any)
		expect(res).toMatchObject({ status_code: 200, success: true })
		const loginCall = httpSpy.mock.calls.find((c) =>
			String(c[0]?.url).includes('/api/v1/auth/login')
		)
		expect(loginCall).toBeDefined()
		expect(loginCall?.[0]?.headers?.authorization).toBeUndefined()
		expect(loginCall?.[0]?.headers?.cookie).toBeUndefined()
	})

	it('should execute performTokenRefresh statelessly per event and set access_token_exp from expires_in fallback', async () => {
		const clusters: ICluster[] = [
			{
				key: 'cluster-a',
				name: 'cluster-a',
				url: 'http://cluster-a:8080',
				clientPath: '/cluster-a'
			}
		]
		vi.spyOn(clusterUtils, 'getClustersConfig').mockReturnValue(clusters)

		const forwardSpy = vi.spyOn(clusterUtils, 'forwardToCluster').mockResolvedValue({
			status: 200,
			headers: new Headers(),
			data: {
				access_token: 'opaque-stateless-at',
				refresh_token: 'opaque-stateless-rt',
				expires_in: 600
			}
		})

		const { performTokenRefresh } = await import('@@/server/services/auth-refresh.service')
		const event1 = createMockEvent(
			undefined,
			'refresh_token=stateless-rt-a; auth_provider=local'
		)
		const event2 = createMockEvent(
			undefined,
			'refresh_token=stateless-rt-b; auth_provider=local'
		)

		const [res1, res2] = await Promise.all([
			performTokenRefresh(event1 as any, {
				refreshToken: 'stateless-rt-a',
				authProvider: 'local',
				clusterHeader: '/cluster-a'
			}),
			performTokenRefresh(event2 as any, {
				refreshToken: 'stateless-rt-b',
				authProvider: 'local',
				clusterHeader: '/cluster-a'
			})
		])

		expect(forwardSpy).toHaveBeenCalledTimes(2)
		expect(res1.ok).toBe(true)
		expect(res2.ok).toBe(true)
		expect((event1.context as any)._refreshedAccessToken).toBe('opaque-stateless-at')
		expect((event2.context as any)._refreshedAccessToken).toBe('opaque-stateless-at')
		expect(String(event1.node.res.getHeader('set-cookie'))).toContain(
			'access_token=opaque-stateless-at'
		)
		expect(String(event1.node.res.getHeader('set-cookie'))).toContain('access_token_exp=')
		expect(String(event1.node.res.getHeader('set-cookie'))).toContain('auth_refreshed_at=')
	})

	it('clearAuthCookies clears known auth cookies and dynamic patrolli_*/auth_* cookies while preserving unrelated cookies', async () => {
		const { clearAuthCookies } = await import('@@/server/utils/auth-cookies')
		const event = createMockEvent(
			undefined,
			'access_token=at; patrolli_cluster1=sess1; auth_state=xyz; theme_preference=dark'
		)

		clearAuthCookies(event as any)

		const setCookies = event.node.res.getHeader('set-cookie')
		const cookieStr = Array.isArray(setCookies) ? setCookies.join('\n') : String(setCookies)
		expect(cookieStr).toContain('access_token=;')
		expect(cookieStr).toContain('access_token_exp=;')
		expect(cookieStr).toContain('auth_refreshed_at=;')
		expect(cookieStr).toContain('patrolli_cluster1=;')
		expect(cookieStr).toContain('auth_state=;')
		expect(cookieStr).not.toContain('theme_preference=;')
	})
})

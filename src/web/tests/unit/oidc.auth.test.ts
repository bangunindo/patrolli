// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { beforeEach, describe, expect, it, vi } from 'vitest'
import * as clusterService from '@@/server/services/cluster.service'
import * as clusterAccessService from '@@/server/services/cluster-access.service'
import * as sslService from '@@/server/services/ssl.service'
import * as oidcService from '@@/server/modules/oidc/oidc.service'
import {
	handleLegacyKeycloakCallback,
	handleOidcCallback,
	handleOidcLogin,
	handleOidcPublicConfig
} from '@@/server/modules/oidc/oidc.controller'
import { normalizeAuthProvider } from '@@/shared/utils/auth'
import {
	encodeSignedOidcTransaction,
	formatOidcConnectionError,
	getValidatedOidcConfig,
	OIDC_TX_COOKIE_NAME,
	resetOidcTxSigningKeyWarning,
	resolveDynamicOidcRedirectUri,
	selectUpstreamAuthTokens
} from '@@/server/modules/oidc/oidc.utils'
import type { IOidcConfig } from '@@/shared/types/cluster.type'

const { undiciFetchMock, agentCloseSpy, agentOptionsSpy } = vi.hoisted(() => ({
	undiciFetchMock: vi.fn(),
	agentCloseSpy: vi.fn().mockResolvedValue(undefined),
	agentOptionsSpy: vi.fn()
}))

vi.mock('undici', async (importOriginal) => {
	const actual = await importOriginal<typeof import('undici')>()
	class MockAgent extends actual.Agent {
		constructor(options?: any) {
			super(options)
			agentOptionsSpy(options)
		}

		override close(): Promise<void> {
			agentCloseSpy()
			return Promise.resolve()
		}
	}
	return {
		...actual,
		Agent: MockAgent,
		fetch: (...args: any[]) => undiciFetchMock(...args)
	}
})

function createMockGetEvent(url: string, cookie?: string, headers: Record<string, string> = {}) {
	const headersMap = new Map<string, any>()
	let statusCode = 200
	return {
		method: 'GET',
		path: url,
		context: {},
		node: {
			req: {
				url,
				headers: {
					host: 'localhost:3000',
					...(cookie ? { cookie } : {}),
					...headers
				}
			},
			res: {
				get statusCode() {
					return statusCode
				},
				set statusCode(code: number) {
					statusCode = code
				},
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
					if (!existing) {
						headersMap.set(k, val)
					} else if (Array.isArray(existing)) {
						headersMap.set(k, [...existing, ...(Array.isArray(val) ? val : [val])])
					} else {
						headersMap.set(k, [existing, ...(Array.isArray(val) ? val : [val])])
					}
				}
			}
		}
	}
}

const baseOidcConfig: IOidcConfig = {
	issuer: 'https://idp.example.com/realms/patrolli',
	client_id: 'patrolli-web',
	client_secret: 'secret',
	redirect_uri: '/auth/oidc/callback',
	scopes: 'openid profile email',
	token_source: 'id_token',
	display_name: 'Corporate SSO'
}

describe('Generic OIDC Module', () => {
	beforeEach(() => {
		vi.restoreAllMocks()
	})

	describe('selectUpstreamAuthTokens', () => {
		it('selects id_token when token_source is id_token', () => {
			const result = selectUpstreamAuthTokens(
				{
					access_token: 'opaque-at',
					id_token: 'signed-id-token',
					refresh_token: 'rt-1',
					expires_in: 300
				},
				'id_token'
			)
			expect(result.access_token).toBe('signed-id-token')
			expect(result.id_token).toBe('signed-id-token')
			expect(result.refresh_token).toBe('rt-1')
		})

		it('selects access_token when token_source is access_token', () => {
			const result = selectUpstreamAuthTokens(
				{
					access_token: 'jwt-at',
					id_token: 'signed-id-token',
					refresh_token: 'rt-1'
				},
				'access_token'
			)
			expect(result.access_token).toBe('jwt-at')
			expect(result.id_token).toBe('signed-id-token')
		})

		it('syncs expires_in from id_token JWT exp when token_source is id_token', () => {
			const expSecs = Math.floor(Date.now() / 1000) + 120
			const payloadB64 = Buffer.from(JSON.stringify({ exp: expSecs })).toString('base64url')
			const idJwt = `hdr.${payloadB64}.sig`

			const result = selectUpstreamAuthTokens(
				{
					access_token: 'opaque-at',
					id_token: idJwt,
					refresh_token: 'rt-1',
					expires_in: 3600
				},
				'id_token'
			)
			expect(result.access_token).toBe(idJwt)
			expect(result.expires_in).toBeDefined()
			expect(result.expires_in!).toBeLessThanOrEqual(120)
			expect(result.expires_in!).toBeGreaterThan(100)
		})

		it('falls back to JWT access_token when token_source is id_token and id_token is omitted', () => {
			const payloadB64 = Buffer.from(JSON.stringify({ sub: 'u1' })).toString('base64url')
			const jwtAt = `hdr.${payloadB64}.sig`

			const result = selectUpstreamAuthTokens(
				{
					access_token: jwtAt,
					refresh_token: 'rt-1'
				},
				'id_token'
			)
			expect(result.access_token).toBe(jwtAt)
		})

		it('returns undefined access_token when token_source is id_token, id_token is omitted, and access_token is opaque', () => {
			const result = selectUpstreamAuthTokens(
				{
					access_token: 'opaque-access-token',
					refresh_token: 'rt-1'
				},
				'id_token'
			)
			expect(result.access_token).toBeUndefined()
		})
	})

	describe('OIDC utility validation and error formatting', () => {
		it('does not append [object Object] when error.cause is a plain object', () => {
			const err = {
				error: 'invalid_grant',
				error_description: 'Session not active',
				cause: { error: 'invalid_grant', error_description: 'Session not active' }
			}
			const formatted = formatOidcConnectionError(err)
			expect(formatted).toBe('Session not active')
			expect(formatted).not.toContain('[object Object]')
		})

		it('rejects invalid issuer URL, missing openid scope, and invalid redirect_uri in getValidatedOidcConfig', () => {
			vi.spyOn(clusterService, 'getOidcConfig').mockReturnValue({
				...baseOidcConfig,
				issuer: 'not-a-valid-url'
			})
			expect(getValidatedOidcConfig().errorPayload?.error?.detail).toContain('valid http')

			vi.spyOn(clusterService, 'getOidcConfig').mockReturnValue({
				...baseOidcConfig,
				scopes: 'profile email'
			})
			expect(getValidatedOidcConfig().errorPayload?.error?.detail).toContain('"openid"')

			for (const badRedirectUri of [
				'https://app.example.com/custom-callback',
				'https://app.example.com/auth/oidc/callback?foo=bar',
				'https://app.example.com/auth/oidc/callback#frag',
				'https://user:pass@app.example.com/auth/oidc/callback',
				'/wrong/callback'
			]) {
				vi.spyOn(clusterService, 'getOidcConfig').mockReturnValue({
					...baseOidcConfig,
					redirect_uri: badRedirectUri
				})
				expect(getValidatedOidcConfig().errorPayload?.error?.detail).toContain(
					'OIDC redirect_uri'
				)
			}

			vi.spyOn(clusterService, 'getOidcConfig').mockReturnValue({
				...baseOidcConfig,
				redirect_uri: 'https://app.example.com/auth/oidc/callback'
			})
			expect(getValidatedOidcConfig().config).toBeDefined()

			vi.spyOn(clusterService, 'getOidcConfig').mockReturnValue({
				...baseOidcConfig,
				redirect_uri: '/auth/oidc/callback'
			})
			expect(getValidatedOidcConfig().config).toBeDefined()
		})

		it('prioritizes explicit absolute redirect_uri over request headers, rejects invalid configured redirect_uri, and falls back dynamically when omitted or relative', () => {
			const headerEvent = createMockGetEvent('/api/auth/oidc/login', undefined, {
				'x-forwarded-host': 'proxy.example.com',
				'x-forwarded-proto': 'https',
				host: 'localhost:3000'
			})
			expect(
				resolveDynamicOidcRedirectUri(
					headerEvent as any,
					'https://pinned.example.com/auth/oidc/callback'
				)
			).toBe('https://pinned.example.com/auth/oidc/callback')

			expect(
				resolveDynamicOidcRedirectUri(
					headerEvent as any,
					'https://pinned.example.com/wrong/path'
				)
			).toBeUndefined()

			expect(
				resolveDynamicOidcRedirectUri(
					headerEvent as any,
					'https://pinned.example.com/auth/oidc/callback?foo=bar'
				)
			).toBeUndefined()

			expect(resolveDynamicOidcRedirectUri(headerEvent as any, '/auth/oidc/callback')).toBe(
				'https://proxy.example.com/auth/oidc/callback'
			)

			const noHostEvent = createMockGetEvent('/api/auth/oidc/login')
			delete (noHostEvent.node.req.headers as any).host
			expect(
				resolveDynamicOidcRedirectUri(noHostEvent as any, '/auth/oidc/callback')
			).toBeUndefined()
			expect(
				resolveDynamicOidcRedirectUri(
					noHostEvent as any,
					'https://app.example.com/auth/oidc/callback'
				)
			).toBe('https://app.example.com/auth/oidc/callback')
		})

		it('warns once in production when falling back to ephemeral OIDC transaction signing key', () => {
			const prevNodeEnv = process.env.NODE_ENV
			const prevSecret = process.env.OIDC_COOKIE_SECRET
			process.env.NODE_ENV = 'production'
			delete process.env.OIDC_COOKIE_SECRET
			resetOidcTxSigningKeyWarning()

			const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {})
			const publicConfig: IOidcConfig = {
				...baseOidcConfig,
				client_secret: undefined
			}
			const tx = {
				codeVerifier: 'v1',
				state: 's1',
				nonce: 'n1',
				redirectUri: 'https://app.example.com/auth/oidc/callback'
			}

			try {
				encodeSignedOidcTransaction(tx, publicConfig)
				encodeSignedOidcTransaction(tx, publicConfig)
				expect(warnSpy).toHaveBeenCalledTimes(1)
				expect(warnSpy).toHaveBeenCalledWith(expect.stringContaining('OIDC_COOKIE_SECRET'))
			} finally {
				resetOidcTxSigningKeyWarning()
				process.env.NODE_ENV = prevNodeEnv
				if (prevSecret === undefined) {
					delete process.env.OIDC_COOKIE_SECRET
				} else {
					process.env.OIDC_COOKIE_SECRET = prevSecret
				}
			}
		})

		it('resolves first protocol token from multi-hop x-forwarded-proto header', () => {
			const multiHopEvent = createMockGetEvent('/api/auth/oidc/login', undefined, {
				'x-forwarded-host': 'patrolli.example.com',
				'x-forwarded-proto': 'https, http'
			})
			expect(resolveDynamicOidcRedirectUri(multiHopEvent as any)).toBe(
				'https://patrolli.example.com/auth/oidc/callback'
			)

			const invalidProtoEvent = createMockGetEvent('/api/auth/oidc/login', undefined, {
				'x-forwarded-host': 'patrolli.example.com',
				'x-forwarded-proto': 'ftp'
			})
			expect(resolveDynamicOidcRedirectUri(invalidProtoEvent as any)).toBe(
				'http://patrolli.example.com/auth/oidc/callback'
			)
		})

		it('rejects malformed x-forwarded-host values and falls back to valid host header or fallbackUri', () => {
			const injectedPathEvent = createMockGetEvent('/api/auth/oidc/login', undefined, {
				'x-forwarded-host': 'evil.example.com/injected',
				host: 'patrolli.local:3000'
			})
			expect(resolveDynamicOidcRedirectUri(injectedPathEvent as any)).toBe(
				'http://patrolli.local:3000/auth/oidc/callback'
			)

			const userInfoEvent = createMockGetEvent('/api/auth/oidc/login', undefined, {
				'x-forwarded-host': 'user@evil.example.com',
				host: 'bad host/path'
			})
			expect(resolveDynamicOidcRedirectUri(userInfoEvent as any)).toBeUndefined()
			expect(
				resolveDynamicOidcRedirectUri(userInfoEvent as any, '/auth/oidc/callback')
			).toBeUndefined()
			expect(
				resolveDynamicOidcRedirectUri(
					userInfoEvent as any,
					'https://fallback.example.com/auth/oidc/callback'
				)
			).toBe('https://fallback.example.com/auth/oidc/callback')

			const multiHopHostEvent = createMockGetEvent('/api/auth/oidc/login', undefined, {
				'x-forwarded-host': 'app.example.com:8443, internal-proxy:8080',
				'x-forwarded-proto': 'https'
			})
			expect(resolveDynamicOidcRedirectUri(multiHopHostEvent as any)).toBe(
				'https://app.example.com:8443/auth/oidc/callback'
			)
		})
	})

	describe('handleOidcPublicConfig', () => {
		it('returns enabled true, display_name, and Cache-Control header when OIDC is configured', () => {
			vi.spyOn(clusterService, 'getOidcConfig').mockReturnValue(baseOidcConfig)
			const event = createMockGetEvent('/api/auth/oidc/config')
			const res = handleOidcPublicConfig(event as any)
			expect(res).toEqual({
				status_code: 200,
				success: true,
				data: {
					enabled: true,
					display_name: 'Corporate SSO'
				}
			})
			expect(event.node.res.getHeader('cache-control')).toBe('public, max-age=60')
		})

		it('returns enabled false when OIDC is not configured', () => {
			vi.spyOn(clusterService, 'getOidcConfig').mockReturnValue(null)
			const res = handleOidcPublicConfig()
			expect(res).toEqual({
				status_code: 200,
				success: true,
				data: {
					enabled: false,
					display_name: 'SSO'
				}
			})
		})

		it('logs a warning when OIDC config is present but invalid', () => {
			vi.spyOn(clusterService, 'getOidcConfig').mockReturnValue({
				...baseOidcConfig,
				issuer: 'not-a-valid-url'
			})
			const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {})

			const res = handleOidcPublicConfig()
			expect(res.data.enabled).toBe(false)
			expect(warnSpy).toHaveBeenCalledWith(
				expect.stringContaining('Disabling OIDC login due to invalid config')
			)
		})
	})

	describe('handleOidcLogin', () => {
		it('generates authorization URL and stores PKCE/state/nonce in HttpOnly cookie', async () => {
			vi.spyOn(clusterService, 'getOidcConfig').mockReturnValue(baseOidcConfig)
			vi.spyOn(oidcService, 'createOidcAuthorizationRequest').mockResolvedValue({
				url: 'https://idp.example.com/authorize?client_id=patrolli-web&code_challenge=abc&state=st1&nonce=nc1',
				transaction: {
					codeVerifier: 'verifier-123',
					state: 'st1',
					nonce: 'nc1',
					redirectUri: 'http://localhost:3000/auth/oidc/callback'
				}
			})

			const event = createMockGetEvent('/api/auth/oidc/login')
			const res = await handleOidcLogin(event as any)

			expect(res).toMatchObject({
				status_code: 200,
				success: true,
				data: {
					url: expect.stringContaining('https://idp.example.com/authorize')
				}
			})

			const setCookies = event.node.res.getHeader('set-cookie')
			const cookieStr = Array.isArray(setCookies) ? setCookies.join('\n') : String(setCookies)
			expect(cookieStr).toContain(`${OIDC_TX_COOKIE_NAME}=`)
			expect(cookieStr).toContain('HttpOnly')
		})
	})

	describe('handleOidcCallback', () => {
		it('rejects callback when oidc_tx cookie is missing', async () => {
			vi.spyOn(clusterService, 'getOidcConfig').mockReturnValue(baseOidcConfig)
			const event = createMockGetEvent('/auth/oidc/callback?code=abc&state=st1')
			const html = await handleOidcCallback(event as any)

			expect(event.node.res.statusCode).toBe(400)
			expect(html).toContain('OIDC login session expired or invalid')
		})

		it('rejects callback when oidc_tx cookie is tampered or unsigned', async () => {
			vi.spyOn(clusterService, 'getOidcConfig').mockReturnValue(baseOidcConfig)
			const unsignedCookie = Buffer.from(
				JSON.stringify({
					codeVerifier: 'verifier-123',
					state: 'st1',
					nonce: 'nc1',
					redirectUri: 'http://localhost:3000/auth/oidc/callback'
				}),
				'utf-8'
			).toString('base64url')

			const unsignedEvent = createMockGetEvent(
				'/auth/oidc/callback?code=auth-code-1&state=st1',
				`${OIDC_TX_COOKIE_NAME}=${unsignedCookie}`
			)
			await handleOidcCallback(unsignedEvent as any)
			expect(unsignedEvent.node.res.statusCode).toBe(400)

			const validSigned = encodeSignedOidcTransaction(
				{
					codeVerifier: 'verifier-123',
					state: 'st1',
					nonce: 'nc1',
					redirectUri: 'http://localhost:3000/auth/oidc/callback'
				},
				baseOidcConfig
			)
			const tamperedSigEvent = createMockGetEvent(
				'/auth/oidc/callback?code=auth-code-1&state=st1',
				`${OIDC_TX_COOKIE_NAME}=${validSigned.slice(0, -2)}AA`
			)
			await handleOidcCallback(tamperedSigEvent as any)
			expect(tamperedSigEvent.node.res.statusCode).toBe(400)
		})

		it('rejects callback when signed oidc_tx contains an invalid callback pathname', async () => {
			vi.spyOn(clusterService, 'getOidcConfig').mockReturnValue(baseOidcConfig)
			const invalidPathSigned = encodeSignedOidcTransaction(
				{
					codeVerifier: 'verifier-123',
					state: 'st1',
					nonce: 'nc1',
					redirectUri: 'https://evil.example.com/not-callback'
				},
				baseOidcConfig
			)

			const event = createMockGetEvent(
				'/auth/oidc/callback?code=auth-code-1&state=st1',
				`${OIDC_TX_COOKIE_NAME}=${invalidPathSigned}`
			)
			const html = await handleOidcCallback(event as any)
			expect(event.node.res.statusCode).toBe(400)
			expect(html).toContain('OIDC login session expired or invalid')
		})

		it('rejects callback when signed oidc_tx is expired past expiresAt', async () => {
			vi.spyOn(clusterService, 'getOidcConfig').mockReturnValue(baseOidcConfig)
			const expiredSigned = encodeSignedOidcTransaction(
				{
					codeVerifier: 'verifier-123',
					state: 'st1',
					nonce: 'nc1',
					redirectUri: 'http://localhost:3000/auth/oidc/callback',
					expiresAt: Date.now() - 1000
				},
				baseOidcConfig
			)

			const event = createMockGetEvent(
				'/auth/oidc/callback?code=auth-code-1&state=st1',
				`${OIDC_TX_COOKIE_NAME}=${expiredSigned}`
			)
			const html = await handleOidcCallback(event as any)
			expect(event.node.res.statusCode).toBe(400)
			expect(html).toContain('OIDC login session expired or invalid')
		})

		it('supports OIDC_COOKIE_SECRET env override for signing oidc_tx cookies', async () => {
			const prevSecret = process.env.OIDC_COOKIE_SECRET
			process.env.OIDC_COOKIE_SECRET = 'shared-cluster-cookie-secret'
			try {
				vi.spyOn(clusterService, 'getOidcConfig').mockReturnValue(baseOidcConfig)
				vi.spyOn(clusterAccessService, 'verifyUserClusterAccess').mockResolvedValue(
					[] as any
				)
				vi.spyOn(oidcService, 'exchangeOidcAuthorizationCode').mockResolvedValue({
					access_token: 'idp-access-token',
					id_token: 'idp-id-token',
					refresh_token: 'idp-refresh-token',
					expires_in: 600
				})

				const txCookieValue = encodeSignedOidcTransaction(
					{
						codeVerifier: 'verifier-123',
						state: 'st1',
						nonce: 'nc1',
						redirectUri: 'http://localhost:3000/auth/oidc/callback'
					},
					baseOidcConfig
				)
				const event = createMockGetEvent(
					'/auth/oidc/callback?code=auth-code-1&state=st1',
					`${OIDC_TX_COOKIE_NAME}=${txCookieValue}`
				)
				await handleOidcCallback(event as any)
				expect(event.node.res.statusCode).toBe(200)
			} finally {
				if (prevSecret === undefined) {
					delete process.env.OIDC_COOKIE_SECRET
				} else {
					process.env.OIDC_COOKIE_SECRET = prevSecret
				}
			}
		})

		it('handles duplicate error query parameters without throwing', async () => {
			const event = createMockGetEvent(
				'/auth/oidc/callback?error=access_denied&error=second_error'
			)
			const html = await handleOidcCallback(event as any)

			expect(event.node.res.statusCode).toBe(400)
			expect(html).toContain('access_denied')
		})

		it('returns 400 when authorizationCodeGrant fails with OIDC client validation error', async () => {
			vi.spyOn(clusterService, 'getOidcConfig').mockReturnValue(baseOidcConfig)
			const validationErr: any = new Error('unexpected JSON attribute value encountered')
			validationErr.code = 'OAUTH_JSON_ATTRIBUTE_COMPARISON_FAILED'
			vi.spyOn(oidcService, 'exchangeOidcAuthorizationCode').mockRejectedValue(validationErr)

			const txCookieValue = encodeSignedOidcTransaction(
				{
					codeVerifier: 'verifier-123',
					state: 'st1',
					nonce: 'nc1',
					redirectUri: 'http://localhost:3000/auth/oidc/callback'
				},
				baseOidcConfig
			)

			const event = createMockGetEvent(
				'/auth/oidc/callback?code=auth-code-1&state=wrong-state',
				`${OIDC_TX_COOKIE_NAME}=${txCookieValue}`
			)

			const html = await handleOidcCallback(event as any)
			expect(event.node.res.statusCode).toBe(400)
			expect(html).toContain('unexpected JSON attribute value encountered')
		})

		it('exchanges code, verifies transaction cookie, and sets oidc session cookies', async () => {
			vi.spyOn(clusterService, 'getOidcConfig').mockReturnValue(baseOidcConfig)
			vi.spyOn(clusterAccessService, 'verifyUserClusterAccess').mockResolvedValue([] as any)
			const exchangeSpy = vi
				.spyOn(oidcService, 'exchangeOidcAuthorizationCode')
				.mockResolvedValue({
					access_token: 'idp-access-token',
					id_token: 'idp-id-token',
					refresh_token: 'idp-refresh-token',
					expires_in: 600
				})

			const txCookieValue = encodeSignedOidcTransaction(
				{
					codeVerifier: 'verifier-123',
					state: 'st1',
					nonce: 'nc1',
					redirectUri: 'http://localhost:3000/auth/oidc/callback'
				},
				baseOidcConfig
			)

			const event = createMockGetEvent(
				'/auth/oidc/callback?code=auth-code-1&state=st1',
				`${OIDC_TX_COOKIE_NAME}=${txCookieValue}`
			)

			const html = await handleOidcCallback(event as any)

			expect(event.node.res.statusCode).toBe(200)
			expect(html).toContain('Authenticating...')
			expect(exchangeSpy).toHaveBeenCalledTimes(1)

			const setCookies = event.node.res.getHeader('set-cookie')
			const cookieStr = Array.isArray(setCookies) ? setCookies.join('\n') : String(setCookies)
			expect(cookieStr).toContain('access_token=idp-id-token')
			expect(cookieStr).toContain('refresh_token=idp-refresh-token')
			expect(cookieStr).toContain('auth_provider=oidc')
		})
	})

	describe('resolveOidcClientAuth', () => {
		it('auto-selects client_secret_basic when discovery only supports client_secret_basic', () => {
			const auth = oidcService.resolveOidcClientAuth(baseOidcConfig)
			const body = new URLSearchParams()
			const headers = new Headers()

			auth(
				{
					issuer: baseOidcConfig.issuer,
					token_endpoint_auth_methods_supported: ['client_secret_basic']
				} as any,
				{ client_id: baseOidcConfig.client_id } as any,
				body,
				headers
			)

			expect(headers.get('authorization')).toMatch(/^Basic /)
			expect(body.get('client_secret')).toBeNull()
		})

		it('uses client_secret_post by default when supported or unspecified', () => {
			const auth = oidcService.resolveOidcClientAuth(baseOidcConfig)
			const body = new URLSearchParams()
			const headers = new Headers()

			auth(
				{
					issuer: baseOidcConfig.issuer,
					token_endpoint_auth_methods_supported: [
						'client_secret_basic',
						'client_secret_post'
					]
				} as any,
				{ client_id: baseOidcConfig.client_id } as any,
				body,
				headers
			)

			expect(headers.get('authorization')).toBeNull()
			expect(body.get('client_id')).toBe(baseOidcConfig.client_id)
			expect(body.get('client_secret')).toBe(baseOidcConfig.client_secret)
		})

		it('honors explicit token_endpoint_auth_method override', () => {
			const auth = oidcService.resolveOidcClientAuth({
				...baseOidcConfig,
				token_endpoint_auth_method: 'client_secret_basic'
			})
			const body = new URLSearchParams()
			const headers = new Headers()

			auth(
				{
					issuer: baseOidcConfig.issuer,
					token_endpoint_auth_methods_supported: [
						'client_secret_basic',
						'client_secret_post'
					]
				} as any,
				{ client_id: baseOidcConfig.client_id } as any,
				body,
				headers
			)

			expect(headers.get('authorization')).toMatch(/^Basic /)
			expect(body.get('client_secret')).toBeNull()
		})
	})

	describe('acquireOidcClientConfiguration single-flight & cache', () => {
		function createDiscoveryResponse(issuer = baseOidcConfig.issuer): Response {
			return new Response(
				JSON.stringify({
					issuer,
					authorization_endpoint: `${issuer}/authorize`,
					token_endpoint: `${issuer}/token`,
					jwks_uri: `${issuer}/jwks`
				}),
				{
					status: 200,
					headers: { 'content-type': 'application/json' }
				}
			)
		}

		beforeEach(() => {
			oidcService.clearOidcDiscoveryCache()
			undiciFetchMock.mockReset()
			agentCloseSpy.mockClear()
			agentOptionsSpy.mockClear()
		})

		it('deduplicates concurrent discovery requests and re-fetches after cache clear', async () => {
			let resolveFetch!: (value: Response) => void
			undiciFetchMock.mockImplementationOnce(
				() =>
					new Promise<Response>((resolve) => {
						resolveFetch = resolve
					})
			)

			const p1 = oidcService.acquireOidcClientConfiguration(baseOidcConfig)
			const p2 = oidcService.acquireOidcClientConfiguration(baseOidcConfig)

			resolveFetch(createDiscoveryResponse())

			const [h1, h2] = await Promise.all([p1, p2])
			try {
				expect(h1.configuration).toBe(h2.configuration)
				expect(undiciFetchMock).toHaveBeenCalledTimes(1)
				expect(agentOptionsSpy).toHaveBeenCalledWith(
					expect.objectContaining({
						headersTimeout: oidcService.OIDC_HEADERS_TIMEOUT_MS,
						bodyTimeout: oidcService.OIDC_BODY_TIMEOUT_MS,
						connect: expect.objectContaining({
							timeout: oidcService.OIDC_CONNECT_TIMEOUT_MS
						})
					})
				)
			} finally {
				h1.release()
				h2.release()
			}

			oidcService.clearOidcDiscoveryCache()
			undiciFetchMock.mockResolvedValueOnce(createDiscoveryResponse())

			const h3 = await oidcService.acquireOidcClientConfiguration(baseOidcConfig)
			try {
				expect(h3.configuration).not.toBe(h1.configuration)
				expect(undiciFetchMock).toHaveBeenCalledTimes(2)
			} finally {
				h3.release()
			}
		})

		it('invalidates discovery cache when TLS CA fingerprint changes in-place', async () => {
			const tlsKeySpy = vi.spyOn(sslService, 'getTlsConnectCacheKey')
			tlsKeySpy.mockReturnValue('true|/ca.pem:100:50')
			undiciFetchMock.mockResolvedValueOnce(createDiscoveryResponse())

			const h1 = await oidcService.acquireOidcClientConfiguration(baseOidcConfig)
			const h2 = await oidcService.acquireOidcClientConfiguration(baseOidcConfig)
			try {
				expect(h1.configuration).toBe(h2.configuration)
				expect(undiciFetchMock).toHaveBeenCalledTimes(1)
			} finally {
				h1.release()
				h2.release()
			}

			tlsKeySpy.mockReturnValue('true|/ca.pem:200:60')
			undiciFetchMock.mockResolvedValueOnce(createDiscoveryResponse())

			const h3 = await oidcService.acquireOidcClientConfiguration(baseOidcConfig)
			try {
				expect(h3.configuration).not.toBe(h1.configuration)
				expect(undiciFetchMock).toHaveBeenCalledTimes(2)
			} finally {
				h3.release()
			}
		})

		it('does not populate cache and closes agent on release when clearOidcDiscoveryCache is called while discovery is in-flight', async () => {
			let resolveFirst!: (value: Response) => void
			undiciFetchMock.mockImplementationOnce(
				() =>
					new Promise<Response>((resolve) => {
						resolveFirst = resolve
					})
			)

			const inFlight = oidcService.acquireOidcClientConfiguration(baseOidcConfig)
			oidcService.clearOidcDiscoveryCache()

			resolveFirst(createDiscoveryResponse())
			const firstHandle = await inFlight
			firstHandle.release()
			expect(agentCloseSpy).toHaveBeenCalledTimes(1)

			undiciFetchMock.mockResolvedValueOnce(createDiscoveryResponse())

			const secondHandle = await oidcService.acquireOidcClientConfiguration(baseOidcConfig)
			try {
				expect(secondHandle.configuration).not.toBe(firstHandle.configuration)
				expect(undiciFetchMock).toHaveBeenCalledTimes(2)
			} finally {
				secondHandle.release()
			}
		})

		it('closes uncached undici.Agent after caller finishes when cache is cleared in-flight', async () => {
			let resolveFirst!: (value: Response) => void
			undiciFetchMock.mockImplementationOnce(
				() =>
					new Promise<Response>((resolve) => {
						resolveFirst = resolve
					})
			)

			const inFlight = oidcService.createOidcAuthorizationRequest(
				baseOidcConfig,
				'http://localhost:3000/auth/oidc/callback'
			)
			oidcService.clearOidcDiscoveryCache()

			resolveFirst(createDiscoveryResponse())
			const result = await inFlight
			expect(result.url).toContain(`${baseOidcConfig.issuer}/authorize`)
			expect(agentCloseSpy).toHaveBeenCalledTimes(1)
		})

		it('warns once when HTTP issuer is used in production', async () => {
			const prevNodeEnv = process.env.NODE_ENV
			process.env.NODE_ENV = 'production'
			const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {})

			const httpConfig: IOidcConfig = {
				...baseOidcConfig,
				issuer: 'http://idp.internal/realms/patrolli'
			}

			try {
				undiciFetchMock.mockResolvedValueOnce(createDiscoveryResponse(httpConfig.issuer))
				const h1 = await oidcService.acquireOidcClientConfiguration(httpConfig)
				h1.release()
				expect(warnSpy).toHaveBeenCalledTimes(1)
				expect(warnSpy).toHaveBeenCalledWith(
					expect.stringContaining('Insecure HTTP issuer "http://idp.internal"')
				)

				// Force a second discovery without clearing warning state via TTL or different client_id
				undiciFetchMock.mockResolvedValueOnce(createDiscoveryResponse(httpConfig.issuer))
				const h2 = await oidcService.acquireOidcClientConfiguration({
					...httpConfig,
					client_id: 'patrolli-web-2'
				})
				h2.release()
				expect(warnSpy).toHaveBeenCalledTimes(1)
			} finally {
				process.env.NODE_ENV = prevNodeEnv
			}
		})

		it('defers closing cached undici.Agent until active handle is released on cache clear or replacement', async () => {
			undiciFetchMock.mockResolvedValueOnce(createDiscoveryResponse())
			const firstCachedHandle =
				await oidcService.acquireOidcClientConfiguration(baseOidcConfig)

			// Acquire a second handle from the populated cache and release the first
			const secondCachedHandle =
				await oidcService.acquireOidcClientConfiguration(baseOidcConfig)
			firstCachedHandle.release()
			expect(agentCloseSpy).not.toHaveBeenCalled()

			// Clearing cache while secondCachedHandle is still active must NOT close the agent yet
			oidcService.clearOidcDiscoveryCache()
			expect(agentCloseSpy).not.toHaveBeenCalled()

			// Releasing the last active handle closes the retired agent
			secondCachedHandle.release()
			expect(agentCloseSpy).toHaveBeenCalledTimes(1)
		})
	})

	describe('Legacy Keycloak backward compatibility', () => {
		it('normalizes legacy auth_provider "keycloak" to "oidc"', () => {
			expect(normalizeAuthProvider('keycloak')).toBe('oidc')
			expect(normalizeAuthProvider('oidc')).toBe('oidc')
			expect(normalizeAuthProvider('local')).toBe('local')
			expect(normalizeAuthProvider('unknown')).toBeNull()
		})

		it('handles legacy POST /api/auth/keycloak/callback with direct code payload and sets HttpOnly oidc session cookies', async () => {
			vi.spyOn(clusterService, 'getOidcConfig').mockReturnValue({
				...baseOidcConfig,
				token_source: 'access_token',
				display_name: 'Keycloak'
			})
			vi.spyOn(clusterAccessService, 'verifyUserClusterAccess').mockResolvedValue([] as any)
			const jwtPayload = Buffer.from(
				JSON.stringify({
					preferred_username: 'kc-alice',
					sub: 'sub-123',
					email: 'alice@example.com'
				})
			).toString('base64url')
			const mockAccessToken = `hdr.${jwtPayload}.sig`

			const exchangeSpy = vi
				.spyOn(oidcService, 'exchangeOidcAuthorizationCode')
				.mockResolvedValue({
					access_token: mockAccessToken,
					refresh_token: 'kc-rt-secret',
					id_token: 'kc-id-secret',
					expires_in: 300
				})

			const event: any = createMockGetEvent('/api/auth/keycloak/callback')
			event.method = 'POST'
			event.node.req.method = 'POST'
			event.node.req.headers['content-type'] = 'application/json'
			event.node.req[Symbol.for('h3ParsedBody')] = {
				code: 'auth-code-1',
				redirect_uri: 'http://localhost:3000/auth/oidc/callback'
			}

			const result = (await handleLegacyKeycloakCallback(event)) as any
			expect(exchangeSpy).toHaveBeenCalledTimes(1)
			expect(result).toEqual({
				status_code: 200,
				success: true,
				data: {
					expires_in: 300,
					user: {
						preferred_username: 'kc-alice',
						username: 'kc-alice',
						sub: 'sub-123',
						email: 'alice@example.com'
					}
				}
			})

			const setCookies = event.node.res.getHeader('set-cookie')
			const cookieStr = Array.isArray(setCookies) ? setCookies.join('\n') : String(setCookies)
			expect(cookieStr).toContain(`access_token=${mockAccessToken}`)
			expect(cookieStr).toContain('refresh_token=kc-rt-secret')
			expect(cookieStr).toContain('auth_provider=oidc')
		})
	})
})

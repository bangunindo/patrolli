// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { beforeEach, describe, expect, it, vi } from 'vitest'
import * as authRefreshService from '@@/server/services/auth-refresh.service'

function makeJwt(payload: Record<string, any>): string {
	const encoded = Buffer.from(JSON.stringify(payload)).toString('base64url')
	return `hdr.${encoded}.sig`
}

function createMockGetEvent(cookies?: string) {
	const headersMap = new Map<string, any>()
	const req: any = {
		headers: cookies ? { cookie: cookies } : {}
	}
	return {
		method: 'GET',
		context: {},
		node: {
			req,
			res: {
				statusCode: 200,
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
						headersMap.set(
							k,
							Array.isArray(existing) ? [...existing, val] : [existing, val]
						)
					} else {
						headersMap.set(k, val)
					}
				}
			}
		}
	}
}

describe('GET /api/auth/me', () => {
	beforeEach(() => {
		vi.restoreAllMocks()
	})

	it('returns 200 with user claims when access_token is valid and not expired', async () => {
		const validToken = makeJwt({
			preferred_username: 'alice',
			sub: 'user-1',
			exp: Math.floor(Date.now() / 1000) + 300
		})
		const refreshSpy = vi.spyOn(authRefreshService, 'performTokenRefresh')

		const { default: meHandler } = await import('@@/server/api/auth/me.get')
		const event = createMockGetEvent(`access_token=${validToken}; auth_provider=oidc`)

		const res = await meHandler(event as any)
		expect(refreshSpy).not.toHaveBeenCalled()
		expect(event.node.res.getHeader('cache-control')).toBe('private, no-store')
		expect(res).toMatchObject({
			status_code: 200,
			success: true,
			data: {
				user: {
					preferred_username: 'alice',
					username: 'alice',
					sub: 'user-1'
				},
				auth_provider: 'oidc'
			}
		})
	})

	it('refreshes expired access_token transparently and returns 200 with updated user and refreshed id_token claims', async () => {
		const expiredToken = makeJwt({
			preferred_username: 'old-user',
			exp: Math.floor(Date.now() / 1000) - 60
		})
		const expiredIdToken = makeJwt({
			email: 'stale@example.com',
			exp: Math.floor(Date.now() / 1000) - 60
		})
		const freshToken = makeJwt({
			preferred_username: 'bob',
			sub: 'user-2',
			exp: Math.floor(Date.now() / 1000) + 300
		})
		const freshIdToken = makeJwt({
			email: 'bob@example.com',
			exp: Math.floor(Date.now() / 1000) + 300
		})
		const refreshSpy = vi.spyOn(authRefreshService, 'performTokenRefresh').mockResolvedValue({
			ok: true,
			status: 200,
			isSessionInvalid: false,
			tokens: {
				access_token: freshToken,
				id_token: freshIdToken
			}
		})

		const { default: meHandler } = await import('@@/server/api/auth/me.get')
		const event = createMockGetEvent(
			`access_token=${expiredToken}; id_token=${expiredIdToken}; refresh_token=valid-rt; auth_provider=local`
		)

		const res = await meHandler(event as any)
		expect(refreshSpy).toHaveBeenCalledTimes(1)
		expect(res).toMatchObject({
			status_code: 200,
			success: true,
			data: {
				user: {
					preferred_username: 'bob',
					username: 'bob',
					sub: 'user-2',
					email: 'bob@example.com'
				},
				auth_provider: 'local'
			}
		})
	})

	it('discards expired id_token when transparent refresh does not return a new id_token', async () => {
		const expiredToken = makeJwt({
			preferred_username: 'old-user',
			exp: Math.floor(Date.now() / 1000) - 60
		})
		const expiredIdToken = makeJwt({
			email: 'stale@example.com',
			exp: Math.floor(Date.now() / 1000) - 60
		})
		const freshToken = makeJwt({
			preferred_username: 'bob',
			sub: 'user-2',
			exp: Math.floor(Date.now() / 1000) + 300
		})
		vi.spyOn(authRefreshService, 'performTokenRefresh').mockResolvedValue({
			ok: true,
			status: 200,
			isSessionInvalid: false,
			tokens: {
				access_token: freshToken
			}
		})

		const { default: meHandler } = await import('@@/server/api/auth/me.get')
		const event = createMockGetEvent(
			`access_token=${expiredToken}; id_token=${expiredIdToken}; refresh_token=valid-rt; auth_provider=local`
		)

		const res = await meHandler(event as any)
		expect((res as any).data.user.email).toBeUndefined()
		expect((res as any).data.user.preferred_username).toBe('bob')
	})

	it('returns 401 and clears auth cookies when access_token is expired and refresh fails due to invalid session', async () => {
		const expiredToken = makeJwt({
			preferred_username: 'stale-user',
			exp: Math.floor(Date.now() / 1000) - 60
		})
		const refreshSpy = vi.spyOn(authRefreshService, 'performTokenRefresh').mockResolvedValue({
			ok: false,
			status: 401,
			isSessionInvalid: true
		})

		const { default: meHandler } = await import('@@/server/api/auth/me.get')
		const event = createMockGetEvent(
			`access_token=${expiredToken}; refresh_token=invalid-rt; is_authenticated=true; auth_provider=local`
		)

		const res = await meHandler(event as any)
		expect(refreshSpy).toHaveBeenCalledTimes(1)
		expect(event.node.res.statusCode).toBe(401)
		expect(res).toMatchObject({
			status_code: 401,
			success: false,
			is_session_invalid: true
		})
		const setCookieHeader = String(event.node.res.getHeader('set-cookie'))
		expect(setCookieHeader).toContain('is_authenticated=;')
	})

	it('returns 502 without clearing auth cookies when refresh fails due to transient upstream error', async () => {
		const expiredToken = makeJwt({
			preferred_username: 'stale-user',
			exp: Math.floor(Date.now() / 1000) - 60
		})
		const refreshSpy = vi.spyOn(authRefreshService, 'performTokenRefresh').mockResolvedValue({
			ok: false,
			status: 502,
			isSessionInvalid: false,
			errorPayload: {
				status_code: 502,
				success: false,
				error: {
					title: 'Bad Gateway',
					detail: 'Upstream unavailable'
				}
			}
		})

		const { default: meHandler } = await import('@@/server/api/auth/me.get')
		const event = createMockGetEvent(
			`access_token=${expiredToken}; refresh_token=valid-rt; is_authenticated=true; auth_provider=local`
		)

		const res = await meHandler(event as any)
		expect(refreshSpy).toHaveBeenCalledTimes(1)
		expect(event.node.res.statusCode).toBe(502)
		expect(res).toMatchObject({
			status_code: 502,
			success: false
		})
		expect(event.node.res.getHeader('set-cookie')).toBeUndefined()
	})

	it('returns 401 and clears auth cookies when access_token is expired and no refresh_token exists', async () => {
		const expiredToken = makeJwt({
			preferred_username: 'stale-user',
			exp: Math.floor(Date.now() / 1000) - 60
		})

		const { default: meHandler } = await import('@@/server/api/auth/me.get')
		const event = createMockGetEvent(
			`access_token=${expiredToken}; is_authenticated=true; auth_provider=local`
		)

		const res = await meHandler(event as any)
		expect(event.node.res.statusCode).toBe(401)
		expect(event.node.res.getHeader('cache-control')).toBe('private, no-store')
		expect(res).toMatchObject({
			status_code: 401,
			success: false,
			is_session_invalid: true
		})
		const setCookieHeader = String(event.node.res.getHeader('set-cookie'))
		expect(setCookieHeader).toContain('is_authenticated=;')
	})
})

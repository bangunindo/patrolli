// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import securityHeadersMiddleware from '@@/server/middleware/security-headers'
import * as sslUtils from '@@/server/services/ssl.service'

function createMockEvent() {
	const headersMap = new Map<string, string>()
	return {
		headersMap,
		event: {
			node: {
				req: {
					headers: {},
					socket: { encrypted: false }
				},
				res: {
					setHeader: (key: string, val: string) => {
						headersMap.set(key.toLowerCase(), val)
					},
					getHeader: (key: string) => headersMap.get(key.toLowerCase())
				}
			}
		} as any
	}
}

describe('Security Headers Middleware', () => {
	const originalEnv = process.env

	beforeEach(() => {
		vi.restoreAllMocks()
		process.env = { ...originalEnv }
		process.env.CORS_ORIGIN = 'https://localhost:3000, https://kc.localtest.me:8443'
	})

	afterEach(() => {
		process.env = originalEnv
	})

	it('should attach baseline security headers to responses', () => {
		const { event, headersMap } = createMockEvent()
		vi.spyOn(sslUtils, 'isRequestSecure').mockReturnValue(false)

		securityHeadersMiddleware(event)

		expect(headersMap.get('x-content-type-options')).toBe('nosniff')
		expect(headersMap.get('x-frame-options')).toBe('SAMEORIGIN')
		expect(headersMap.get('referrer-policy')).toBe('strict-origin-when-cross-origin')
		expect(headersMap.get('x-xss-protection')).toBe('1; mode=block')
		expect(headersMap.get('x-dns-prefetch-control')).toBe('off')
		expect(headersMap.get('permissions-policy')).toBe(
			'camera=(), microphone=(), geolocation=()'
		)
		expect(headersMap.get('content-security-policy')).toContain("default-src 'self'")
		expect(headersMap.get('content-security-policy')).toContain('connect-src')
		expect(headersMap.get('strict-transport-security')).toBeUndefined()
	})

	it('should attach Strict-Transport-Security (HSTS) when running over HTTPS', () => {
		const { event, headersMap } = createMockEvent()
		vi.spyOn(sslUtils, 'isRequestSecure').mockReturnValue(true)

		securityHeadersMiddleware(event)

		expect(headersMap.get('strict-transport-security')).toBe(
			'max-age=31536000; includeSubDomains'
		)
		expect(headersMap.get('x-content-type-options')).toBe('nosniff')
	})

	it('should handle CORS preflight OPTIONS request for allowed origin', () => {
		const { event, headersMap } = createMockEvent()
		event.method = 'OPTIONS'
		event.node.req.headers = {
			origin: 'https://kc.localtest.me:8443'
		}

		event.node.res.statusCode = 200

		securityHeadersMiddleware(event)

		expect(event.node.res.statusCode).toBe(204)
		expect(headersMap.get('access-control-allow-origin')).toBe('https://kc.localtest.me:8443')
		expect(headersMap.get('access-control-allow-credentials')).toBe('true')
		expect(headersMap.get('access-control-allow-methods')).toContain('GET')
		expect(headersMap.get('access-control-allow-headers')).toContain('Content-Type')
	})

	it('should not attach CORS headers for disallowed origin', () => {
		const { event, headersMap } = createMockEvent()
		event.method = 'GET'
		event.node.req.headers = {
			origin: 'https://evil-unauthorized-site.com'
		}

		securityHeadersMiddleware(event)

		expect(headersMap.get('access-control-allow-origin')).toBeUndefined()
	})
})

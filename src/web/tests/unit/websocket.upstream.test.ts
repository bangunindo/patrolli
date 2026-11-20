// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { beforeEach, describe, expect, it, vi } from 'vitest'
import * as clusterUtils from '@@/server/services/cluster.service'
import * as sslService from '@@/server/services/ssl.service'
import * as authRefreshService from '@@/server/services/auth-refresh.service'
import { isJwtExpired } from '@@/server/utils/auth-cookies'
import {
	clearWebSocketAgentCache,
	createUpstreamWebSocket,
	getWebSocketAgent,
	resolveWebSocketAuthHeader,
	resolveWebSocketUpstreamUrl
} from '@@/server/services/websocket.service'
import type { ICluster } from '@@/shared/types/cluster.type'

describe('WebSocket Upstream URL Resolution', () => {
	beforeEach(() => {
		vi.restoreAllMocks()
		vi.spyOn(clusterUtils, 'getOidcConfig').mockReturnValue(null)
	})
	const defaultCluster: ICluster = {
		name: 'patrolli',
		url: 'http://patrolli:8080'
	}

	it('should strip cluster name prefix and forward /ws/v1 path 1:1', () => {
		const rawUrl = '/patrolli/ws/v1/processors/subscribe_capture/proc-123'
		const { upstreamUrl, upstreamProtocol, fullUpstreamPath } = resolveWebSocketUpstreamUrl(
			defaultCluster,
			rawUrl
		)

		expect(upstreamProtocol).toBe('ws:')
		expect(fullUpstreamPath).toBe('/ws/v1/processors/subscribe_capture/proc-123')
		expect(upstreamUrl).toBe('ws://patrolli:8080/ws/v1/processors/subscribe_capture/proc-123')
	})

	it('should forward any upstream path 1:1 without rewriting', () => {
		const rawUrl = '/patrolli/ws/subscribe'
		const { upstreamUrl, fullUpstreamPath } = resolveWebSocketUpstreamUrl(
			defaultCluster,
			rawUrl
		)

		expect(fullUpstreamPath).toBe('/ws/subscribe')
		expect(upstreamUrl).toBe('ws://patrolli:8080/ws/subscribe')
	})

	it('should handle cluster URL with existing pathname base and forward 1:1', () => {
		const cluster: ICluster = {
			name: 'patrolli',
			url: 'http://patrolli:8080/base'
		}
		const rawUrl = '/patrolli/ws/v1/subscribe'
		const { upstreamUrl, fullUpstreamPath } = resolveWebSocketUpstreamUrl(cluster, rawUrl)

		expect(fullUpstreamPath).toBe('/base/ws/v1/subscribe')
		expect(upstreamUrl).toBe('ws://patrolli:8080/base/ws/v1/subscribe')
	})

	it('should resolve wss protocol for secure cluster URLs', () => {
		const cluster: ICluster = {
			name: 'patrolli',
			url: 'https://patrolli.example.com:8443'
		}
		const rawUrl = '/patrolli/ws/v1/live'
		const { upstreamUrl, upstreamProtocol } = resolveWebSocketUpstreamUrl(cluster, rawUrl)

		expect(upstreamProtocol).toBe('wss:')
		expect(upstreamUrl).toBe('wss://patrolli.example.com:8443/ws/v1/live')
	})

	it('should strip token and access_token from query parameters while preserving other params', () => {
		const rawUrl =
			'/patrolli/ws/v1/capture?token=secret123&access_token=jwt456&filter=metrics&limit=50'
		const { upstreamUrl, fullUpstreamPath } = resolveWebSocketUpstreamUrl(
			defaultCluster,
			rawUrl
		)

		expect(fullUpstreamPath).not.toContain('token=secret123')
		expect(fullUpstreamPath).not.toContain('access_token=jwt456')
		expect(fullUpstreamPath).toContain('filter=metrics')
		expect(fullUpstreamPath).toContain('limit=50')
		expect(upstreamUrl).toContain('/ws/v1/capture?filter=metrics&limit=50')
	})

	it('should strip cluster key when matching by key and forward path 1:1', () => {
		const cluster: ICluster = {
			key: 'pk1',
			name: 'p1',
			url: 'http://patrolli:8080'
		}
		const rawUrl = '/pk1/ws/v1/test'
		const { upstreamUrl, fullUpstreamPath } = resolveWebSocketUpstreamUrl(cluster, rawUrl)

		expect(fullUpstreamPath).toBe('/ws/v1/test')
		expect(upstreamUrl).toBe('ws://patrolli:8080/ws/v1/test')
	})

	it('should extract access_token cookie without triggering server-side refresh and detect expiry', () => {
		const expiredPayload = Buffer.from(
			JSON.stringify({ exp: Math.floor(Date.now() / 1000) - 60 })
		).toString('base64url')
		const expiredJwt = `hdr.${expiredPayload}.sig`

		const refreshSpy = vi.spyOn(authRefreshService, 'performTokenRefresh')

		const authHeader = resolveWebSocketAuthHeader({
			cookie: `access_token=${expiredJwt}; refresh_token=old-rt; auth_provider=oidc; active_cluster_url=/patrolli`
		})

		expect(refreshSpy).not.toHaveBeenCalled()
		expect(authHeader).toBe(`Bearer ${expiredJwt}`)
		expect(isJwtExpired(expiredJwt)).toBe(true)
	})

	it('should reuse cached http/https agents, refresh https agent, and drain idle sockets when TLS key changes', () => {
		clearWebSocketAgentCache()
		const http1 = getWebSocketAgent(false)
		const http2 = getWebSocketAgent(false)
		expect(http1).toBe(http2)

		const keySpy = vi.spyOn(sslService, 'getTlsConnectCacheKey')
		keySpy.mockReturnValue('true|/ca.pem:100:50')
		const https1 = getWebSocketAgent(true)
		const https2 = getWebSocketAgent(true)
		expect(https1).toBe(https2)

		const idleSocketDestroy = vi.fn()
		;(https1.freeSockets as any)['example.com:443'] = [{ destroy: idleSocketDestroy }]

		keySpy.mockReturnValue('true|/ca.pem:200:60')
		const https3 = getWebSocketAgent(true)
		expect(https3).not.toBe(https1)
		expect(idleSocketDestroy).toHaveBeenCalledTimes(1)
		clearWebSocketAgentCache()
	})

	it('should not throw in getWebSocketAgent(true) when getOidcConfig throws', () => {
		clearWebSocketAgentCache()
		vi.spyOn(clusterUtils, 'getOidcConfig').mockImplementation(() => {
			throw new Error('Configuration file not found or empty')
		})

		expect(() => getWebSocketAgent(true)).not.toThrow()
		clearWebSocketAgentCache()
	})

	it('should not hardcode rejectUnauthorized: false in createUpstreamWebSocket', async () => {
		const https = await import('node:https')
		const { EventEmitter } = await import('node:events')

		let capturedOptions: Record<string, any> | undefined
		const requestSpy = vi.spyOn(https.default, 'request').mockImplementation((opts: any) => {
			capturedOptions = opts
			const fakeReq: any = new EventEmitter()
			fakeReq.end = vi.fn()
			fakeReq.destroy = vi.fn()
			fakeReq.setTimeout = vi.fn()
			return fakeReq
		})

		try {
			const agent = getWebSocketAgent(true)
			const ws = createUpstreamWebSocket('wss://127.0.0.1:1/ws/test', {
				headers: { authorization: 'Bearer test' },
				agent
			})
			ws.on('error', () => {})
			ws.terminate()

			expect(requestSpy).toHaveBeenCalledTimes(1)
			expect(capturedOptions?.agent).toBe(agent)
			expect(capturedOptions?.rejectUnauthorized).toBeUndefined()
			expect(capturedOptions?.headers?.authorization).toBe('Bearer test')
		} finally {
			requestSpy.mockRestore()
			clearWebSocketAgentCache()
		}
	})
})

// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { EventEmitter } from 'node:events'
import WebSocket from 'ws'
import * as clusterUtils from '@@/server/services/cluster.service'
import * as sslService from '@@/server/services/ssl.service'
import * as authRefreshService from '@@/server/services/auth-refresh.service'
import { isJwtExpired } from '@@/server/utils/auth-cookies'
import * as wsService from '@@/server/services/websocket.service'
import {
	clearWebSocketAgentCache,
	createUpstreamWebSocket,
	getWebSocketAgent,
	MAX_WS_CLOSE_REASON_BYTES,
	normalizeWebSocketCloseCode,
	normalizeWebSocketCloseReason,
	resolveWebSocketAuthHeader,
	resolveWebSocketUpstreamUrl
} from '@@/server/services/websocket.service'
import {
	createNitroWebSocketProxy,
	MAX_PENDING_WS_MESSAGES
} from '@@/server/modules/websocket/websocket.controller'
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

	it('should route pipeline processor group preview endpoint across distinct clusters via Nitro BFF', () => {
		const clusterA: ICluster = {
			key: 'cluster-a',
			name: 'cluster-a',
			url: 'http://patrolli-a:8080'
		}
		const clusterB: ICluster = {
			key: 'cluster-b',
			name: 'cluster-b',
			url: 'https://patrolli-b.internal:8443/core'
		}

		const pathA = '/cluster-a/ws/v1/pipelines/pipe-1/canvases/canvas-1/nodes/node-1/preview'
		const resolvedA = resolveWebSocketUpstreamUrl(clusterA, pathA)
		expect(resolvedA.upstreamProtocol).toBe('ws:')
		expect(resolvedA.fullUpstreamPath).toBe(
			'/ws/v1/pipelines/pipe-1/canvases/canvas-1/nodes/node-1/preview'
		)
		expect(resolvedA.upstreamUrl).toBe(
			'ws://patrolli-a:8080/ws/v1/pipelines/pipe-1/canvases/canvas-1/nodes/node-1/preview'
		)

		const pathB = '/cluster-b/ws/v1/pipelines/pipe-2/canvases/canvas-2/nodes/node-2/preview'
		const resolvedB = resolveWebSocketUpstreamUrl(clusterB, pathB)
		expect(resolvedB.upstreamProtocol).toBe('wss:')
		expect(resolvedB.fullUpstreamPath).toBe(
			'/core/ws/v1/pipelines/pipe-2/canvases/canvas-2/nodes/node-2/preview'
		)
		expect(resolvedB.upstreamUrl).toBe(
			'wss://patrolli-b.internal:8443/core/ws/v1/pipelines/pipe-2/canvases/canvas-2/nodes/node-2/preview'
		)
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

		const webHeaders = new Headers({
			cookie: `access_token=${expiredJwt}; auth_provider=local`
		})
		const authFromWebHeaders = resolveWebSocketAuthHeader({
			headers: webHeaders,
			cookie: (webHeaders as any).cookie
		})
		expect(authFromWebHeaders).toBe(`Bearer ${expiredJwt}`)
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

	it('should normalize reserved WebSocket close codes (1004, 1005, 1006, 1015) to 1000', () => {
		expect(normalizeWebSocketCloseCode(1000)).toBe(1000)
		expect(normalizeWebSocketCloseCode(1001)).toBe(1001)
		expect(normalizeWebSocketCloseCode(1011)).toBe(1011)
		expect(normalizeWebSocketCloseCode(4401)).toBe(4401)

		expect(normalizeWebSocketCloseCode(1004)).toBe(1000)
		expect(normalizeWebSocketCloseCode(1005)).toBe(1000)
		expect(normalizeWebSocketCloseCode(1006)).toBe(1000)
		expect(normalizeWebSocketCloseCode(1015)).toBe(1000)
		expect(normalizeWebSocketCloseCode(undefined)).toBe(1000)
		expect(normalizeWebSocketCloseCode(999)).toBe(1000)
		expect(normalizeWebSocketCloseCode(5000)).toBe(1000)
	})

	it('should truncate WebSocket close reason safely at UTF-8 character boundaries up to 123 bytes', () => {
		expect(normalizeWebSocketCloseReason(undefined)).toBeUndefined()
		expect(normalizeWebSocketCloseReason('')).toBeUndefined()
		expect(normalizeWebSocketCloseReason(Buffer.from(''))).toBeUndefined()

		const shortReason = 'Normal closure'
		expect(normalizeWebSocketCloseReason(shortReason)).toBe(shortReason)

		const asciiLong = 'a'.repeat(150)
		const truncatedAscii = normalizeWebSocketCloseReason(asciiLong)!
		expect(Buffer.byteLength(truncatedAscii, 'utf-8')).toBe(MAX_WS_CLOSE_REASON_BYTES)
		expect(truncatedAscii).toBe('a'.repeat(MAX_WS_CLOSE_REASON_BYTES))

		// Multi-byte UTF-8: 'a' (1 byte) + 50 * 'é' (2 bytes each) = 101 bytes, or 62 * 'é' = 124 bytes
		const twoByteBoundary = 'a' + 'é'.repeat(62)
		const truncatedTwoByte = normalizeWebSocketCloseReason(twoByteBoundary)!
		expect(Buffer.byteLength(truncatedTwoByte, 'utf-8')).toBeLessThanOrEqual(
			MAX_WS_CLOSE_REASON_BYTES
		)
		expect(Buffer.byteLength(truncatedTwoByte, 'utf-8')).toBe(123)
		expect(truncatedTwoByte).not.toContain('\uFFFD')

		// 3-byte UTF-8 character cut mid-sequence at byte 123 (e.g. 121 'a' + '€' (3 bytes) = 124 bytes)
		const threeByteMidCut = 'a'.repeat(121) + '€'
		const truncatedThreeByte = normalizeWebSocketCloseReason(threeByteMidCut)!
		expect(Buffer.byteLength(truncatedThreeByte, 'utf-8')).toBe(121)
		expect(truncatedThreeByte).toBe('a'.repeat(121))
		expect(truncatedThreeByte).not.toContain('\uFFFD')
	})

	it('should cap pendingMessages at MAX_PENDING_WS_MESSAGES (drop newest) while CONNECTING and flush on open', () => {
		vi.spyOn(clusterUtils, 'getClustersConfig').mockReturnValue([defaultCluster])

		const fakeUpstream: any = new EventEmitter()
		fakeUpstream.readyState = WebSocket.CONNECTING
		fakeUpstream.send = vi.fn()
		fakeUpstream.close = vi.fn()

		vi.spyOn(wsService, 'createUpstreamWebSocket').mockReturnValue(fakeUpstream)

		const handler = createNitroWebSocketProxy() as any
		const hooks = handler.__websocket__ || handler
		const peer = {
			id: 'peer-queue-test',
			request: {
				url: '/patrolli/ws/v1/pipelines/p1/canvases/c1/nodes/n1/preview',
				headers: {}
			},
			send: vi.fn(),
			close: vi.fn()
		}

		hooks.open?.(peer)

		for (let i = 1; i <= MAX_PENDING_WS_MESSAGES + 10; i++) {
			hooks.message?.(peer, { text: () => `msg-${i}` })
		}

		expect(fakeUpstream.send).not.toHaveBeenCalled()

		fakeUpstream.readyState = WebSocket.OPEN
		fakeUpstream.emit('open')

		expect(fakeUpstream.send).toHaveBeenCalledTimes(MAX_PENDING_WS_MESSAGES)
		expect(fakeUpstream.send).toHaveBeenNthCalledWith(1, 'msg-1')
		expect(fakeUpstream.send).toHaveBeenNthCalledWith(
			MAX_PENDING_WS_MESSAGES,
			`msg-${MAX_PENDING_WS_MESSAGES}`
		)

		hooks.close?.(peer, { code: 1000, reason: 'done' })
		expect(fakeUpstream.close).toHaveBeenCalledTimes(1)
	})

	it('should close upstream WebSocket when client disconnects or errors while upstream is still CONNECTING', () => {
		vi.spyOn(clusterUtils, 'getClustersConfig').mockReturnValue([defaultCluster])

		const fakeUpstreamClose: any = new EventEmitter()
		fakeUpstreamClose.readyState = WebSocket.CONNECTING
		fakeUpstreamClose.send = vi.fn()
		fakeUpstreamClose.close = vi.fn()

		const fakeUpstreamError: any = new EventEmitter()
		fakeUpstreamError.readyState = WebSocket.CONNECTING
		fakeUpstreamError.send = vi.fn()
		fakeUpstreamError.close = vi.fn()

		const createSpy = vi
			.spyOn(wsService, 'createUpstreamWebSocket')
			.mockReturnValueOnce(fakeUpstreamClose)
			.mockReturnValueOnce(fakeUpstreamError)

		const handler = createNitroWebSocketProxy() as any
		const hooks = handler.__websocket__ || handler

		const peer1 = {
			id: 'peer-early-close',
			request: { url: '/patrolli/ws/v1/preview', headers: {} },
			send: vi.fn(),
			close: vi.fn()
		}
		hooks.open?.(peer1)
		hooks.close?.(peer1, { code: 1001, reason: 'going away' })
		expect(fakeUpstreamClose.close).toHaveBeenCalledTimes(1)
		expect(() => fakeUpstreamClose.emit('error', new Error('aborted'))).not.toThrow()

		const peer2 = {
			id: 'peer-early-error',
			request: { url: '/patrolli/ws/v1/preview', headers: {} },
			send: vi.fn(),
			close: vi.fn()
		}
		hooks.open?.(peer2)
		hooks.error?.(peer2, new Error('client error'))
		expect(fakeUpstreamError.close).toHaveBeenCalledTimes(1)
		expect(createSpy).toHaveBeenCalledTimes(2)
	})
})

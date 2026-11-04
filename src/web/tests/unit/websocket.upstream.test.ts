// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from 'vitest'
import { resolveWebSocketUpstreamUrl } from '@@/server/services/websocket.service'
import type { ICluster } from '@@/shared/types/cluster.type'

describe('WebSocket Upstream URL Resolution', () => {
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
})

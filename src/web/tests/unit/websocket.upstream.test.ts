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

	it('should maintain /ws path when path_websocket is not configured', () => {
		const rawUrl = '/patrolli/ws/processors/subscribe_capture/proc-123'
		const { upstreamUrl, upstreamProtocol, fullUpstreamPath } = resolveWebSocketUpstreamUrl(
			defaultCluster,
			rawUrl
		)

		expect(upstreamProtocol).toBe('ws:')
		expect(fullUpstreamPath).toBe('/ws/processors/subscribe_capture/proc-123')
		expect(upstreamUrl).toBe('ws://patrolli:8080/ws/processors/subscribe_capture/proc-123')
	})

	it('should replace /ws prefix with path_websocket (e.g. api/v1/ws)', () => {
		const cluster: ICluster = {
			name: 'patrolli',
			url: 'http://patrolli:8080',
			path_websocket: 'api/v1/ws'
		}
		const rawUrl = '/patrolli/ws/processors/subscribe_capture/proc-123'
		const { upstreamUrl, fullUpstreamPath } = resolveWebSocketUpstreamUrl(cluster, rawUrl)

		expect(fullUpstreamPath).toBe('/api/v1/ws/processors/subscribe_capture/proc-123')
		expect(upstreamUrl).toBe(
			'ws://patrolli:8080/api/v1/ws/processors/subscribe_capture/proc-123'
		)
	})

	it('should replace /ws prefix with ws/v1 when path_websocket is ws/v1', () => {
		const cluster: ICluster = {
			name: 'patrolli',
			url: 'http://patrolli:8080',
			path_websocket: 'ws/v1'
		}
		const rawUrl = '/patrolli/ws/processors/subscribe_capture/proc-123'
		const { upstreamUrl, fullUpstreamPath } = resolveWebSocketUpstreamUrl(cluster, rawUrl)

		expect(fullUpstreamPath).toBe('/ws/v1/processors/subscribe_capture/proc-123')
		expect(upstreamUrl).toBe('ws://patrolli:8080/ws/v1/processors/subscribe_capture/proc-123')
	})

	it('should sanitize leading and trailing slashes in path_websocket', () => {
		const cluster: ICluster = {
			name: 'patrolli',
			url: 'http://patrolli:8080',
			path_websocket: '/api/v1/ws/'
		}
		const rawUrl = '/patrolli/ws/processors/subscribe_capture/proc-123'
		const { upstreamUrl, fullUpstreamPath } = resolveWebSocketUpstreamUrl(cluster, rawUrl)

		expect(fullUpstreamPath).toBe('/api/v1/ws/processors/subscribe_capture/proc-123')
		expect(upstreamUrl).toBe(
			'ws://patrolli:8080/api/v1/ws/processors/subscribe_capture/proc-123'
		)
	})

	it('should handle cluster URL with existing pathname base and rewrite path_websocket', () => {
		const cluster: ICluster = {
			name: 'patrolli',
			url: 'http://patrolli:8080/base',
			path_websocket: 'api/v1/ws'
		}
		const rawUrl = '/patrolli/ws/subscribe'
		const { upstreamUrl, fullUpstreamPath } = resolveWebSocketUpstreamUrl(cluster, rawUrl)

		expect(fullUpstreamPath).toBe('/base/api/v1/ws/subscribe')
		expect(upstreamUrl).toBe('ws://patrolli:8080/base/api/v1/ws/subscribe')
	})

	it('should resolve wss protocol and https agent for secure cluster URLs', () => {
		const cluster: ICluster = {
			name: 'patrolli',
			url: 'https://patrolli.example.com:8443',
			path_websocket: 'api/v1/ws'
		}
		const rawUrl = '/patrolli/ws/live'
		const { upstreamUrl, upstreamProtocol } = resolveWebSocketUpstreamUrl(cluster, rawUrl)

		expect(upstreamProtocol).toBe('wss:')
		expect(upstreamUrl).toBe('wss://patrolli.example.com:8443/api/v1/ws/live')
	})

	it('should strip token and access_token from query parameters while preserving other params', () => {
		const cluster: ICluster = {
			name: 'patrolli',
			url: 'http://patrolli:8080',
			path_websocket: 'api/v1/ws'
		}
		const rawUrl =
			'/patrolli/ws/capture?token=secret123&access_token=jwt456&filter=metrics&limit=50'
		const { upstreamUrl, fullUpstreamPath } = resolveWebSocketUpstreamUrl(cluster, rawUrl)

		expect(fullUpstreamPath).not.toContain('token=secret123')
		expect(fullUpstreamPath).not.toContain('access_token=jwt456')
		expect(fullUpstreamPath).toContain('filter=metrics')
		expect(fullUpstreamPath).toContain('limit=50')
		expect(upstreamUrl).toContain('/api/v1/ws/capture?filter=metrics&limit=50')
	})

	it('should handle whitespace-only path_websocket by falling back to raw path', () => {
		const cluster: ICluster = {
			name: 'patrolli',
			url: 'http://patrolli:8080',
			path_websocket: '    '
		}
		const rawUrl = '/patrolli/ws/test'
		const { upstreamUrl } = resolveWebSocketUpstreamUrl(cluster, rawUrl)

		expect(upstreamUrl).toBe('ws://patrolli:8080/ws/test')
	})

	it('should prepend path_websocket if path does not start with /ws', () => {
		const cluster: ICluster = {
			name: 'patrolli',
			url: 'http://patrolli:8080',
			path_websocket: 'api/v1/ws'
		}
		const rawUrl = '/patrolli/events/stream'
		const { upstreamUrl, fullUpstreamPath } = resolveWebSocketUpstreamUrl(cluster, rawUrl)

		expect(fullUpstreamPath).toBe('/api/v1/ws/events/stream')
		expect(upstreamUrl).toBe('ws://patrolli:8080/api/v1/ws/events/stream')
	})

	it('should not duplicate path_websocket prefix if path already starts with path_websocket', () => {
		const clusterApiWs: ICluster = {
			name: 'patrolli',
			url: 'http://patrolli:8080',
			path_websocket: 'api/v1/ws'
		}
		const res1 = resolveWebSocketUpstreamUrl(clusterApiWs, '/patrolli/api/v1/ws/live')
		expect(res1.fullUpstreamPath).toBe('/api/v1/ws/live')
		expect(res1.upstreamUrl).toBe('ws://patrolli:8080/api/v1/ws/live')

		const clusterWsV1: ICluster = {
			name: 'patrolli',
			url: 'http://patrolli:8080',
			path_websocket: 'ws/v1'
		}
		const res2 = resolveWebSocketUpstreamUrl(clusterWsV1, '/patrolli/ws/v1/live')
		expect(res2.fullUpstreamPath).toBe('/ws/v1/live')
		expect(res2.upstreamUrl).toBe('ws://patrolli:8080/ws/v1/live')
	})
})

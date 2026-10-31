// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import WebSocket from 'ws'
import { defineWebSocketHandler } from 'h3'
import { getClustersConfig } from '@@/server/services/cluster.service'
import {
	createUpstreamWebSocket,
	resolveWebSocketAuthHeader,
	resolveWebSocketUpstreamUrl
} from '@@/server/services/websocket.service'
import type { INitroWsSession } from './websocket.type'

/**
 * In-memory registry of active client peer connections to upstream WebSocket instances.
 */
const nitroWsSessions = new Map<string, INitroWsSession>()

/**
 * Creates a Nitro defineWebSocketHandler for CrossWS.
 * Handles WebSocket routing, cluster matching, token forwarding, and bidirectional message proxying.
 */
export function createNitroWebSocketProxy() {
	return defineWebSocketHandler({
		upgrade(req) {
			console.log(`[Nitro WS Upgrade] Handshake requested: ${req.url}`)
		},

		open(peer: any) {
			const rawUrl = peer.request?.url || ''
			console.log(`[Nitro WS Server] Client connected (ID: ${peer.id}, URL: ${rawUrl})`)

			try {
				const allClusters = getClustersConfig()
				if (!allClusters || allClusters.length === 0) {
					console.warn(
						'[Nitro WS Proxy] No clusters configured to proxy WebSocket request'
					)
					peer.close(1011, 'No clusters configured')
					return
				}

				let parsedPathname = rawUrl
				let parsedSearch = ''
				try {
					const parsed = new URL(rawUrl, 'http://localhost:3000')
					parsedPathname = parsed.pathname
					parsedSearch = parsed.search
				} catch {
					// fallback
				}

				const decodedPath = decodeURIComponent(parsedPathname)
				const targetCluster = allClusters.find(
					(c) =>
						(c.name && decodedPath.startsWith(`/${c.name}/`)) ||
						(c.key && decodedPath.startsWith(`/${c.key}/`)) ||
						(c.clientPath && decodedPath.startsWith(`${c.clientPath}/`))
				)

				if (!targetCluster) {
					console.warn(
						`[Nitro WS Proxy] Target cluster not found for path: ${decodedPath}`
					)
					peer.close(1011, 'Target cluster not found')
					return
				}

				const { upstreamUrl, fullUpstreamPath, agent } = resolveWebSocketUpstreamUrl(
					targetCluster,
					rawUrl
				)

				console.log(
					`[Nitro WS Proxy] Routing client "${rawUrl}" -> Backend "${targetCluster.name}" (${upstreamUrl})`
				)

				// Resolve Bearer auth from headers, cookie, or query parameters
				const searchParams = new URLSearchParams(parsedSearch)
				const queryToken =
					searchParams.get('token') || searchParams.get('access_token') || undefined
				const authHeader = resolveWebSocketAuthHeader({
					headers: peer.request?.headers,
					cookie: peer.request?.headers?.cookie,
					queryToken
				})

				const headers: Record<string, string> = {}
				if (peer.request?.headers?.cookie) {
					headers.cookie = peer.request.headers.cookie
				}
				if (authHeader) {
					headers.authorization = authHeader
					console.log(
						`[Nitro WS Proxy] Attached Authorization token for backend cluster: ${targetCluster.name}`
					)
				} else {
					console.warn(
						`[Nitro WS Proxy] Warning: No Authorization token found for WebSocket client ${peer.id}`
					)
				}

				const upstream = createUpstreamWebSocket(upstreamUrl, {
					headers,
					agent
				})

				nitroWsSessions.set(peer.id, {
					upstream,
					targetCluster,
					path: fullUpstreamPath
				})

				upstream.on('open', () => {
					console.log(
						`[Nitro WS Proxy] Connected to backend cluster: ${targetCluster.name} (${upstreamUrl})`
					)
				})

				upstream.on('message', (data: any, isBinary: boolean) => {
					const msgStr = isBinary ? data : data.toString('utf-8')
					console.log(`[WS Message from ${targetCluster.name}]:`, msgStr)
					peer.send(msgStr)
				})

				upstream.on('close', (code: number, reason: Buffer) => {
					console.log(
						`[Nitro WS Proxy] Backend cluster ${targetCluster.name} closed connection (${code}): ${reason ? reason.toString() : ''}`
					)
					nitroWsSessions.delete(peer.id)
					peer.close(code, reason ? reason.toString() : undefined)
				})

				upstream.on('error', (err: Error) => {
					console.error(
						`[Nitro WS Proxy Error] Upstream connection to ${targetCluster.name} failed:`,
						err.message
					)
					nitroWsSessions.delete(peer.id)
					peer.close(1011, 'Upstream Error')
				})
			} catch (err: any) {
				console.error('[Nitro WS Proxy] Failed to initialize upstream:', err.message)
				nitroWsSessions.delete(peer.id)
				peer.close(1011, 'Internal Server Error')
			}
		},

		message(peer: any, message: any) {
			const text = message.text()
			console.log('[WS Client Message]:', text)
			const session = nitroWsSessions.get(peer.id)
			const upstream = session?.upstream
			if (upstream && upstream.readyState === WebSocket.OPEN) {
				upstream.send(message.rawData)
			}
		},

		close(peer: any, details: any) {
			console.log(
				`[Nitro WS Server] Client disconnected (ID: ${peer.id}, Code: ${details?.code}, Reason: ${details?.reason || '-'})`
			)
			const session = nitroWsSessions.get(peer.id)
			if (session?.upstream && session.upstream.readyState === WebSocket.OPEN) {
				session.upstream.close()
			}
			nitroWsSessions.delete(peer.id)
		},

		error(peer: any, error: any) {
			console.error(`[Nitro WS Server Error] Peer ${peer.id}:`, error)
			const session = nitroWsSessions.get(peer.id)
			if (session?.upstream && session.upstream.readyState === WebSocket.OPEN) {
				session.upstream.close()
			}
			nitroWsSessions.delete(peer.id)
		}
	})
}

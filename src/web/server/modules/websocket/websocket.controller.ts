// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import WebSocket from 'ws'
import { defineWebSocketHandler } from 'h3'
import { getClustersConfig } from '@@/server/services/cluster.service'
import { isJwtExpired } from '@@/server/utils/auth-cookies'
import {
	createUpstreamWebSocket,
	normalizeWebSocketCloseCode,
	normalizeWebSocketCloseReason,
	resolveWebSocketAuthHeader,
	resolveWebSocketUpstreamUrl
} from '@@/server/services/websocket.service'
import type { INitroWsSession } from '@@/shared/types/websocket.type'

const nitroWsSessions = new Map<string, INitroWsSession>()

export const MAX_PENDING_WS_MESSAGES = 50

function closeUpstreamSafely(upstream?: WebSocket): void {
	if (
		upstream &&
		(upstream.readyState === WebSocket.OPEN || upstream.readyState === WebSocket.CONNECTING)
	) {
		try {
			// Prevent unhandled error event when closing a socket before the handshake completes
			upstream.on('error', () => {})
			upstream.close()
		} catch {
			// Ignore close errors on connecting or closing sockets
		}
	}
}

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

				const searchParams = new URLSearchParams(parsedSearch)
				const queryToken =
					searchParams.get('token') || searchParams.get('access_token') || undefined
				const authHeader = resolveWebSocketAuthHeader({
					headers: peer.request?.headers,
					cookie:
						peer.request?.headers?.cookie ||
						(typeof peer.request?.headers?.get === 'function'
							? peer.request.headers.get('cookie')
							: undefined),
					queryToken
				})

				const token = authHeader?.startsWith('Bearer ')
					? authHeader.slice(7).trim()
					: authHeader
				if (token && isJwtExpired(token)) {
					console.warn(
						`[Nitro WS Proxy] Access token expired for WebSocket client ${peer.id}; closing so client can refresh`
					)
					peer.close(4401, 'Access token expired')
					return
				}

				const headers: Record<string, string> = {}
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

				const pendingMessages: string[] = []

				nitroWsSessions.set(peer.id, {
					upstream,
					targetCluster,
					path: fullUpstreamPath,
					pendingMessages
				})

				upstream.on('open', () => {
					console.log(
						`[Nitro WS Proxy] Connected to backend cluster: ${targetCluster.name} (${upstreamUrl})`
					)
					while (pendingMessages.length > 0 && upstream.readyState === WebSocket.OPEN) {
						const queued = pendingMessages.shift()
						if (queued !== undefined) {
							upstream.send(queued)
						}
					}
				})

				upstream.on('message', (data: any, isBinary: boolean) => {
					const msgStr = isBinary ? data : data.toString('utf-8')
					peer.send(msgStr)
				})

				upstream.on('close', (code: number, reason: Buffer) => {
					const reasonText = reason ? reason.toString() : ''
					console.log(
						`[Nitro WS Proxy] Backend cluster ${targetCluster.name} closed connection (${code}): ${reasonText}`
					)
					nitroWsSessions.delete(peer.id)
					try {
						peer.close(
							normalizeWebSocketCloseCode(code),
							normalizeWebSocketCloseReason(reasonText)
						)
					} catch {
						// Ignore error if client peer already closed
					}
				})

				upstream.on('error', (err: Error) => {
					console.error(
						`[Nitro WS Proxy Error] Upstream connection to ${targetCluster.name} failed:`,
						err.message
					)
					nitroWsSessions.delete(peer.id)
					try {
						peer.close(1011, 'Upstream Error')
					} catch {
						// Ignore error if client peer already closed
					}
				})
			} catch (err: any) {
				console.error('[Nitro WS Proxy] Failed to initialize upstream:', err.message)
				nitroWsSessions.delete(peer.id)
				try {
					peer.close(1011, 'Internal Server Error')
				} catch {
					// Ignore error if client peer already closed
				}
			}
		},

		message(peer: any, message: any) {
			const text = message.text()
			const session = nitroWsSessions.get(peer.id)
			const upstream = session?.upstream
			if (upstream && upstream.readyState === WebSocket.OPEN) {
				upstream.send(text)
			} else if (upstream && upstream.readyState === WebSocket.CONNECTING) {
				if (
					session?.pendingMessages &&
					session.pendingMessages.length < MAX_PENDING_WS_MESSAGES
				) {
					session.pendingMessages.push(text)
				}
			}
		},

		close(peer: any, details: any) {
			console.log(
				`[Nitro WS Server] Client disconnected (ID: ${peer.id}, Code: ${details?.code}, Reason: ${details?.reason || '-'})`
			)
			const session = nitroWsSessions.get(peer.id)
			closeUpstreamSafely(session?.upstream)
			nitroWsSessions.delete(peer.id)
		},

		error(peer: any, error: any) {
			console.error(`[Nitro WS Server Error] Peer ${peer.id}:`, error)
			const session = nitroWsSessions.get(peer.id)
			closeUpstreamSafely(session?.upstream)
			nitroWsSessions.delete(peer.id)
		}
	})
}

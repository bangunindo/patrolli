// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type http from 'node:http'
import type https from 'node:https'
import WebSocket from 'ws'
import {
	createHttpAgent,
	createHttpsAgent,
	drainAgentIdleSockets,
	resolveOidcCaPath
} from './apibase.service'
import { getTlsConnectCacheKey } from './ssl.service'
import type { ICluster } from '@@/shared/types/cluster.type'

let cachedWsHttpAgent: http.Agent | null = null
let cachedWsHttpsAgent: { key: string; agent: https.Agent } | null = null

export function clearWebSocketAgentCache(): void {
	cachedWsHttpAgent?.destroy()
	cachedWsHttpAgent = null
	cachedWsHttpsAgent?.agent.destroy()
	cachedWsHttpsAgent = null
}

export function getWebSocketAgent(isHttpsOrWss: boolean): https.Agent | http.Agent {
	if (!isHttpsOrWss) {
		if (!cachedWsHttpAgent) {
			cachedWsHttpAgent = createHttpAgent()
		}
		return cachedWsHttpAgent
	}

	const key = getTlsConnectCacheKey([resolveOidcCaPath()])
	if (cachedWsHttpsAgent && cachedWsHttpsAgent.key === key) {
		return cachedWsHttpsAgent.agent
	}

	const prevAgent = cachedWsHttpsAgent?.agent
	const agent = createHttpsAgent()
	cachedWsHttpsAgent = { key, agent }
	drainAgentIdleSockets(prevAgent)
	return agent
}

export function resolveWebSocketAuthHeader(options: {
	headers?: Record<string, any>
	cookie?: string | string[]
	queryToken?: string
}): string | undefined {
	const existing =
		options.headers?.authorization ||
		options.headers?.Authorization ||
		(typeof options.headers?.get === 'function' && options.headers.get('authorization'))

	if (existing && typeof existing === 'string') {
		return existing.startsWith('Bearer ') ? existing : `Bearer ${existing}`
	}

	const headerCookie =
		typeof options.headers?.cookie === 'string'
			? options.headers.cookie
			: typeof options.headers?.get === 'function'
				? options.headers.get('cookie') || ''
				: ''

	const rawCookie =
		typeof options.cookie === 'string'
			? options.cookie
			: Array.isArray(options.cookie)
				? options.cookie.join('; ')
				: headerCookie

	if (rawCookie) {
		const match = rawCookie.match(/(?:^|;\s*)access_token=([^;]+)/)
		if (match && match[1]) {
			return `Bearer ${decodeURIComponent(match[1])}`
		}
	}

	if (options.queryToken) {
		return `Bearer ${decodeURIComponent(options.queryToken)}`
	}

	return undefined
}

export function resolveWebSocketUpstreamUrl(
	targetCluster: ICluster,
	rawClientUrl: string
): {
	upstreamUrl: string
	upstreamProtocol: 'ws:' | 'wss:'
	fullUpstreamPath: string
	agent: https.Agent | http.Agent
} {
	let parsedPathname = rawClientUrl
	let parsedSearch = ''
	try {
		const parsed = new URL(rawClientUrl, 'http://localhost:3000')
		parsedPathname = parsed.pathname
		parsedSearch = parsed.search
	} catch {
		// Fall back to rawClientUrl when URL parsing fails
	}

	const decodedPath = decodeURIComponent(parsedPathname)

	let upstreamPath = parsedPathname
	if (
		targetCluster.name &&
		(decodedPath.startsWith(`/${targetCluster.name}/`) ||
			upstreamPath.startsWith(`/${targetCluster.name}/`))
	) {
		upstreamPath = upstreamPath.replace(
			new RegExp(`^/${encodeURIComponent(targetCluster.name)}|^/${targetCluster.name}`),
			''
		)
	} else if (targetCluster.key && upstreamPath.startsWith(`/${targetCluster.key}/`)) {
		upstreamPath = upstreamPath.replace(new RegExp(`^/${targetCluster.key}`), '')
	}

	if (!upstreamPath.startsWith('/')) {
		upstreamPath = `/${upstreamPath}`
	}

	// Remove token/access_token from query string forwarded to cluster to prevent leaking credentials in logs
	let cleanSearch = parsedSearch
	if (parsedSearch) {
		const searchParams = new URLSearchParams(parsedSearch)
		searchParams.delete('token')
		searchParams.delete('access_token')
		const qs = searchParams.toString()
		cleanSearch = qs ? `?${qs}` : ''
	}

	const targetUrl = new URL(targetCluster.url)
	const basePath = targetUrl.pathname.replace(/\/+$/, '')
	const fullUpstreamPath = `${basePath}${upstreamPath}${cleanSearch}`

	const isSecure = targetUrl.protocol === 'https:' || targetUrl.protocol === 'wss:'
	const upstreamProtocol: 'ws:' | 'wss:' = isSecure ? 'wss:' : 'ws:'
	const upstreamUrl = `${upstreamProtocol}//${targetUrl.host}${fullUpstreamPath}`
	const agent = getWebSocketAgent(isSecure)

	return {
		upstreamUrl,
		upstreamProtocol,
		fullUpstreamPath,
		agent
	}
}

export const MAX_WS_CLOSE_REASON_BYTES = 123

export function normalizeWebSocketCloseCode(code?: number): number {
	if (
		typeof code !== 'number' ||
		!Number.isInteger(code) ||
		code < 1000 ||
		code > 4999 ||
		code === 1004 ||
		code === 1005 ||
		code === 1006 ||
		code === 1015
	) {
		return 1000
	}
	return code
}

export function normalizeWebSocketCloseReason(reason?: string | Buffer): string | undefined {
	if (!reason) return undefined
	const text = Buffer.isBuffer(reason) ? reason.toString('utf-8') : String(reason)
	if (!text) return undefined

	const buf = Buffer.from(text, 'utf-8')
	if (buf.byteLength <= MAX_WS_CLOSE_REASON_BYTES) {
		return text
	}

	let end = MAX_WS_CLOSE_REASON_BYTES
	// Backtrack if cut in the middle of a multi-byte UTF-8 sequence (continuation bytes: 10xxxxxx)
	while (end > 0 && (buf[end]! & 0xc0) === 0x80) {
		end--
	}
	return buf.subarray(0, end).toString('utf-8')
}

export function createUpstreamWebSocket(
	upstreamUrl: string,
	options: {
		headers?: Record<string, string>
		agent?: https.Agent | http.Agent
	} = {}
): WebSocket {
	return new WebSocket(upstreamUrl, {
		headers: options.headers,
		agent: options.agent
	})
}

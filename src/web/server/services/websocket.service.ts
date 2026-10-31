// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import fs from 'node:fs'
import http from 'node:http'
import https from 'node:https'
import path from 'node:path'
import WebSocket from 'ws'
import { getKeycloakConfig } from './cluster.service'
import { getSslConfig } from './ssl.service'
import type { ICluster } from '@@/shared/types/cluster.type'

/**
 * Creates an HTTPS Agent for upstream WebSocket connections
 * configured with SSL certificates and verification rules from config-fe.yaml
 */
export function createWebSocketHttpsAgent(): https.Agent {
	const sslConfig = getSslConfig()
	const kcConfig = getKeycloakConfig()

	const rejectUnauthorized =
		sslConfig?.reject_unauthorized !== undefined
			? sslConfig.reject_unauthorized
			: process.env.NODE_ENV === 'production'

	const caList: string[] = []
	const caCandidatePaths = [sslConfig?.ca, kcConfig?.ca_cert_path].filter(Boolean) as string[]

	for (const candidate of caCandidatePaths) {
		const resolvedPath = path.isAbsolute(candidate)
			? candidate
			: path.resolve(process.cwd(), candidate)
		try {
			if (fs.existsSync(resolvedPath)) {
				caList.push(fs.readFileSync(resolvedPath, 'utf-8'))
			}
		} catch {
			// Skip candidate if unreadable
		}
	}

	return new https.Agent({
		rejectUnauthorized,
		keepAlive: true,
		ca: caList.length > 0 ? caList : undefined
	})
}

/**
 * Creates an HTTP Agent for upstream WebSocket connections
 */
export function createWebSocketHttpAgent(): http.Agent {
	return new http.Agent({
		keepAlive: true
	})
}

/**
 * Resolves the appropriate Agent for an upstream WebSocket URL
 */
export function getWebSocketAgent(isHttpsOrWss: boolean): https.Agent | http.Agent {
	return isHttpsOrWss ? createWebSocketHttpsAgent() : createWebSocketHttpAgent()
}

/**
 * Resolves Bearer authorization header from incoming request headers, cookies, or query parameters
 */
export function resolveWebSocketAuthHeader(options: {
	headers?: Record<string, any>
	cookie?: string | string[]
	queryToken?: string
}): string | undefined {
	// 1. Existing Authorization header
	const existing =
		options.headers?.authorization ||
		options.headers?.Authorization ||
		(typeof options.headers?.get === 'function' && options.headers.get('authorization'))

	if (existing && typeof existing === 'string') {
		return existing.startsWith('Bearer ') ? existing : `Bearer ${existing}`
	}

	// 2. Cookie access_token
	const rawCookie =
		typeof options.cookie === 'string'
			? options.cookie
			: Array.isArray(options.cookie)
				? options.cookie.join('; ')
				: typeof options.headers?.cookie === 'string'
					? options.headers.cookie
					: ''

	if (rawCookie) {
		const match = rawCookie.match(/(?:^|;\s*)access_token=([^;]+)/)
		if (match && match[1]) {
			return `Bearer ${decodeURIComponent(match[1])}`
		}
	}

	// 3. Query string token
	if (options.queryToken) {
		return `Bearer ${decodeURIComponent(options.queryToken)}`
	}

	return undefined
}

/**
 * Resolves the upstream URL, stripped path, clean query string, and agent for a target cluster
 */
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
		// fallback
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
	const fullUpstreamPath = `${basePath ? `${basePath}${upstreamPath}` : upstreamPath}${cleanSearch}`

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

/**
 * Base factory to create an upstream WebSocket client connection
 */
export function createUpstreamWebSocket(
	upstreamUrl: string,
	options: {
		headers?: Record<string, string>
		agent?: https.Agent | http.Agent
	} = {}
): WebSocket {
	return new WebSocket(upstreamUrl, {
		headers: options.headers,
		agent: options.agent,
		rejectUnauthorized: false
	})
}

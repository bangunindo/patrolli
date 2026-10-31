// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { H3Event } from 'h3'
import { getRequestHeader, setResponseHeaders, setResponseStatus } from 'h3'
import { getAppConfigFileContent } from './cluster.service'
import type { ICorsConfig } from '@@/shared/types/cluster.type'

/**
 * Loads CORS configuration from config-fe.yaml or environment variables.
 */
export function getCorsConfig(configPath?: string): ICorsConfig | null {
	const fileConfig = getAppConfigFileContent(configPath)
	if (fileConfig?.parsed?.cors) {
		const c = fileConfig.parsed.cors
		return {
			enabled: c.enabled !== false,
			origin: c.origin || c.origins,
			methods: c.methods,
			allow_headers: c.allow_headers || c.headers,
			expose_headers: c.expose_headers,
			credentials: c.credentials !== false,
			max_age: c.max_age || 86400
		}
	}

	if (process.env.CORS_ORIGIN || process.env.CORS_ORIGINS) {
		const raw = (process.env.CORS_ORIGIN || process.env.CORS_ORIGINS)!
		const origins = raw.includes(',') ? raw.split(',').map((s) => s.trim()) : raw.trim()
		return {
			enabled: true,
			origin: origins,
			credentials: true,
			max_age: 86400
		}
	}

	return null
}

/**
 * Validates if the requesting origin is allowed based on configured CORS rules.
 */
export function isOriginAllowed(requestOrigin: string, allowedOrigin?: string | string[]): boolean {
	if (!requestOrigin || !allowedOrigin) return false
	if (allowedOrigin === '*') return true
	if (typeof allowedOrigin === 'string') {
		const list = allowedOrigin.includes(',')
			? allowedOrigin.split(',').map((s) => s.trim())
			: [allowedOrigin.trim()]
		return list.includes(requestOrigin) || list.includes('*')
	}
	if (Array.isArray(allowedOrigin)) {
		return allowedOrigin.includes(requestOrigin) || allowedOrigin.includes('*')
	}
	return false
}

/**
 * Evaluates CORS for the current request.
 * Sets Access-Control-* headers and handles OPTIONS preflight requests.
 * Returns true if the request was an OPTIONS preflight that was completed.
 */
export function handleCustomCors(event: H3Event, corsConfig?: ICorsConfig | null): boolean {
	const config = corsConfig ?? getCorsConfig()
	if (!config || config.enabled === false) {
		return false
	}

	const requestOrigin = getRequestHeader(event, 'origin')
	if (!requestOrigin) {
		return false
	}

	const allowed = isOriginAllowed(requestOrigin, config.origin)
	if (!allowed) {
		return false
	}

	const methods = Array.isArray(config.methods)
		? config.methods.join(', ')
		: config.methods || 'GET, HEAD, PUT, PATCH, POST, DELETE, OPTIONS'

	const allowHeaders = Array.isArray(config.allow_headers)
		? config.allow_headers.join(', ')
		: config.allow_headers ||
			'Content-Type, Authorization, X-Cluster-Url, Accept, X-Requested-With, Origin'

	const exposeHeaders = Array.isArray(config.expose_headers)
		? config.expose_headers.join(', ')
		: config.expose_headers || 'Set-Cookie, Content-Length, Content-Range'

	const headers: Record<string, string> = {
		'Access-Control-Allow-Origin': requestOrigin,
		'Access-Control-Allow-Methods': methods,
		'Access-Control-Allow-Headers': allowHeaders,
		'Access-Control-Expose-Headers': exposeHeaders,
		Vary: 'Origin'
	}

	if (config.credentials !== false) {
		headers['Access-Control-Allow-Credentials'] = 'true'
	}

	if (config.max_age) {
		headers['Access-Control-Max-Age'] = String(config.max_age)
	}

	setResponseHeaders(event, headers)

	if (event.method === 'OPTIONS') {
		setResponseStatus(event, 204)
		return true
	}

	return false
}

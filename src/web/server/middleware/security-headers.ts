// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { defineEventHandler, setResponseHeaders } from 'h3'
import { isRequestSecure } from '@@/server/services/ssl.service'
import { handleCustomCors } from '@@/server/services/cors.service'

/**
 * Nitro Middleware to attach standard HTTP Security Headers and handle CORS.
 */
export default defineEventHandler((event) => {
	const cspDirectives = [
		"default-src 'self'",
		"script-src 'self' 'unsafe-inline' 'unsafe-eval'",
		"style-src 'self' 'unsafe-inline' https://fonts.googleapis.com",
		"font-src 'self' data: https://fonts.gstatic.com",
		"img-src 'self' data: blob: https:",
		"connect-src 'self' ws: wss: https: http:",
		"frame-ancestors 'self'",
		"base-uri 'self'",
		"form-action 'self'"
	].join('; ')

	const headers: Record<string, string> = {
		'X-Content-Type-Options': 'nosniff',
		'X-Frame-Options': 'SAMEORIGIN',
		'Referrer-Policy': 'strict-origin-when-cross-origin',
		'X-XSS-Protection': '1; mode=block',
		'X-DNS-Prefetch-Control': 'off',
		'Permissions-Policy': 'camera=(), microphone=(), geolocation=()',
		'Content-Security-Policy': cspDirectives
	}

	// Attach HSTS when served over HTTPS / TLS
	if (isRequestSecure(event)) {
		headers['Strict-Transport-Security'] = 'max-age=31536000; includeSubDomains'
	}

	setResponseHeaders(event, headers)

	// Handle CORS headers and preflight OPTIONS request
	const isPreflightDone = handleCustomCors(event)
	if (isPreflightDone) {
		return ''
	}
})

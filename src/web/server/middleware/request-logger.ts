// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { defineEventHandler, getQuery, getRequestHeaders, getRequestURL } from 'h3'
import { logApiIncoming, logApiResponse } from '@@/server/services/logger.service'

export default defineEventHandler((event) => {
	const url = getRequestURL(event)
	const pathname = url.pathname

	// Only log server api and auth routes to avoid noisy static asset logs
	if (pathname.startsWith('/api') || pathname.startsWith('/auth')) {
		const method = event.method
		const reqHeaders = getRequestHeaders(event)
		const query = getQuery(event)
		const clusterHeader =
			(reqHeaders['x-cluster-url'] as string | undefined) ||
			(query.cluster as string | undefined) ||
			null

		const startTime = Date.now()
		logApiIncoming(method, pathname, clusterHeader, query)

		if (event.node?.res) {
			event.node.res.once('finish', () => {
				const statusCode = event.node.res.statusCode
				const durationMs = Date.now() - startTime
				logApiResponse(method, pathname, statusCode, durationMs)
			})
		}
	}
})

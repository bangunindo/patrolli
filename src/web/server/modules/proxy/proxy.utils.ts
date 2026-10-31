// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { H3Event } from 'h3'
import { getQuery, getRequestHeaders } from 'h3'

/**
 * Extracts target cluster header / query and sanitizes request context
 */
export function extractProxyContext(event: H3Event, pathPrefix: string = '/api/') {
	const method = event.method || 'GET'
	const pathParam = event.context?.params?.path || ''
	const fullApiPath = `${pathPrefix}${pathParam}`

	const reqHeaders = getRequestHeaders(event)
	const query = getQuery(event)
	const clusterHeader =
		(reqHeaders['x-cluster-url'] as string | undefined) ||
		(query.cluster as string | undefined) ||
		null

	return {
		method,
		pathParam,
		fullApiPath,
		reqHeaders,
		query,
		clusterHeader
	}
}

// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { H3Event } from 'h3'
import { setResponseStatus } from 'h3'
import {
	getClustersConfig,
	handleClusterMutation,
	resolveClusterTargets
} from '@@/server/services/cluster.service'
import { distributePagination, rebalancePaginationResponses } from '@@/server/utils/pagination'
import { handleSingleClusterResponse, mergeDynamicResponses } from '@@/server/utils/response'
import { forwardProxyToCluster } from './proxy.api'
import { extractProxyContext } from './proxy.utils'

/**
 * Handles GET /api/[...path] multi-cluster scatter-gather proxy
 */
export async function handleProxyGet(event: H3Event) {
	const { method, fullApiPath, reqHeaders, query, clusterHeader } = extractProxyContext(
		event,
		'/api/'
	)

	const allClusters = getClustersConfig(event)
	const targetClusters = resolveClusterTargets(clusterHeader, allClusters)

	if (targetClusters.length === 0) {
		const errorPayload = {
			status_code: 502,
			success: false,
			error: {
				title: 'Bad Gateway',
				detail: 'No clusters configured. Please configure clusters in config-fe.yaml or environment variables.'
			}
		}
		setResponseStatus(event, 502)
		return errorPayload
	}

	// 1. Single cluster target
	const singleTarget = targetClusters[0]
	if (targetClusters.length === 1 && singleTarget) {
		const res = await forwardProxyToCluster(singleTarget, fullApiPath, {
			method,
			headers: reqHeaders,
			query
		})
		return handleSingleClusterResponse(event, singleTarget, res)
	}

	// 2. Multi-cluster scatter-gather for GET requests
	const limit = query.limit !== undefined ? Number(query.limit) : undefined
	const offset = query.offset !== undefined ? Number(query.offset) : undefined
	const isPaginated =
		typeof limit === 'number' &&
		!isNaN(limit) &&
		typeof offset === 'number' &&
		!isNaN(offset) &&
		targetClusters.length > 1

	if (!isPaginated) {
		const responses = await Promise.all(
			targetClusters.map(async (cluster) => {
				const res = await forwardProxyToCluster(cluster, fullApiPath, {
					method,
					headers: reqHeaders,
					query
				})
				return { cluster, data: res.data, status: res.status }
			})
		)
		const merged = mergeDynamicResponses(responses)
		const finalStatus = merged.status_code || 200
		setResponseStatus(event, finalStatus)
		return merged
	}

	// Paginated scatter-gather
	const nodeCount = targetClusters.length
	const requestPlans = targetClusters.map((cluster, index) => {
		const distributed = distributePagination(offset, limit, nodeCount, index)
		const localQuery = {
			...query,
			offset: distributed.offset,
			limit: distributed.limit
		}
		return {
			cluster,
			localQuery,
			localOffset: distributed.offset,
			localLimit: distributed.limit
		}
	})

	const initialResponses = await Promise.all(
		requestPlans.map(async (plan) => {
			const res = await forwardProxyToCluster(plan.cluster, fullApiPath, {
				method,
				headers: reqHeaders,
				query: plan.localQuery
			})
			return {
				cluster: plan.cluster,
				plan: {
					localOffset: plan.localOffset,
					localLimit: plan.localLimit
				},
				data: res.data,
				status: res.status
			}
		})
	)

	const mergedResult = mergeDynamicResponses(
		initialResponses.map((r) => ({
			cluster: r.cluster,
			data: r.data,
			status: r.status
		}))
	)

	await rebalancePaginationResponses({
		mergedResult,
		initialResponses,
		fullApiPath,
		method,
		reqHeaders,
		query,
		offset,
		limit
	})

	const finalStatus = mergedResult.status_code || 200
	setResponseStatus(event, finalStatus)
	return mergedResult
}

/**
 * Handles mutation methods (POST, PUT, PATCH, DELETE) on /api/[...path]
 */
export async function handleProxyMutation(event: H3Event) {
	return handleClusterMutation(event)
}

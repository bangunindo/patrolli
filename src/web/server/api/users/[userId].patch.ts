// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { defineEventHandler, getQuery, getRequestHeaders, readBody, setResponseStatus } from 'h3'
import {
	forwardToCluster,
	getClustersConfig,
	resolveClusterTargets
} from '@@/server/services/cluster.service'

export default defineEventHandler(async (event) => {
	const userId = event.context.params?.userId
	const reqHeaders = getRequestHeaders(event)
	const query = getQuery(event)
	let body: any = {}

	try {
		body = await readBody(event)
	} catch {
		body = {}
	}

	const clusterHeader =
		(reqHeaders['x-cluster-url'] as string | undefined) ||
		(query.cluster as string | undefined) ||
		null

	const allClusters = getClustersConfig(event)
	const targetClusters = resolveClusterTargets(clusterHeader, allClusters)

	if (targetClusters.length === 0) {
		setResponseStatus(event, 502)
		return {
			status_code: 502,
			success: false,
			error: {
				title: 'Bad Gateway',
				detail: 'No clusters configured.'
			}
		}
	}

	// 1. Single cluster target (standard flow like Host)
	if (targetClusters.length === 1 && targetClusters[0]) {
		const targetCluster = targetClusters[0]
		const patchRes = await forwardToCluster(targetCluster, `/api/users/${userId}`, {
			method: 'PATCH',
			headers: reqHeaders,
			body
		})

		const status = patchRes.status || 200
		setResponseStatus(event, status)
		const data = patchRes.data?.data || patchRes.data || {}
		return {
			status_code: status,
			success: status >= 200 && status < 300,
			data: {
				...data,
				cluster_name: targetCluster.name,
				cluster_url: targetCluster.clientPath,
				clusters: [targetCluster.name]
			}
		}
	}

	// 2. Multi-cluster fallback if multiple target clusters were provided
	const clusterResults = await Promise.all(
		targetClusters.map(async (cluster) => {
			try {
				const patchRes = await forwardToCluster(cluster, `/api/users/${userId}`, {
					method: 'PATCH',
					headers: reqHeaders,
					body
				})
				return {
					cluster,
					success: patchRes.status >= 200 && patchRes.status < 300,
					status: patchRes.status,
					data: patchRes.data?.data || patchRes.data
				}
			} catch (err: any) {
				return {
					cluster,
					success: false,
					status: 500,
					error: err.message
				}
			}
		})
	)

	const successful = clusterResults.filter((r) => r.success)
	if (successful.length === 0) {
		const firstFail = clusterResults[0]
		setResponseStatus(event, firstFail?.status || 500)
		return {
			status_code: firstFail?.status || 500,
			success: false,
			error: {
				title: 'Update Failed',
				detail: 'Failed to update user in target cluster.'
			}
		}
	}

	const primary = successful[0]
	const status = primary?.status || 200
	setResponseStatus(event, status)
	return {
		status_code: status,
		success: true,
		data: {
			...(primary?.data || {}),
			clusters: successful.map((s) => s.cluster.name)
		}
	}
})

// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { defineEventHandler } from 'h3'
import {
	executeClusterRequestsWithAuthRetry,
	executeSingleClusterRequestWithAuthRetry
} from '@@/server/services/auth-refresh.service'
import {
	formatSingleClusterUserMutationResponse,
	readUserRequestBody,
	resolveUserClusterContext,
	respondMultiClusterUserActionResult,
	respondNoClustersConfigured
} from '@@/server/modules/users/users.controller'
import { forwardToCluster } from '@@/server/services/cluster.service'

export default defineEventHandler(async (event) => {
	const userId = event.context.params?.userId
	const { reqHeaders, clusterHeader, targetClusters } = resolveUserClusterContext(event)
	const body = await readUserRequestBody(event)

	if (targetClusters.length === 0) {
		return respondNoClustersConfigured(event)
	}

	if (targetClusters.length === 1 && targetClusters[0]) {
		const targetCluster = targetClusters[0]
		const { result: patchRes } = await executeSingleClusterRequestWithAuthRetry(event, {
			item: targetCluster,
			reqHeaders,
			clusterHeader,
			execute: (cluster, headers) =>
				forwardToCluster(cluster, `/api/v1/users/${userId}`, {
					method: 'PATCH',
					headers,
					body
				})
		})

		return formatSingleClusterUserMutationResponse(event, targetCluster, patchRes, 200)
	}

	const { results: clusterResults } = await executeClusterRequestsWithAuthRetry(event, {
		items: targetClusters,
		reqHeaders,
		clusterHeader,
		execute: async (cluster, headers) => {
			try {
				const patchRes = await forwardToCluster(cluster, `/api/v1/users/${userId}`, {
					method: 'PATCH',
					headers,
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
		}
	})

	return respondMultiClusterUserActionResult(event, {
		results: clusterResults,
		failureTitle: 'Update Failed',
		failureDetail: 'Failed to update user in target cluster.',
		buildSuccess: (successful) => ({
			status: successful[0]?.status || 200,
			data: {
				...(successful[0]?.data || {}),
				clusters: successful.map((s) => s.cluster.name)
			}
		})
	})
})

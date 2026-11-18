// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { defineEventHandler, setResponseStatus } from 'h3'
import { executeSingleClusterRequestWithAuthRetry } from '@@/server/services/auth-refresh.service'
import {
	executeUserActionByUsernameAcrossClusters,
	resolveUserClusterContext,
	respondMultiClusterUserActionResult,
	respondNoClustersConfigured
} from '@@/server/modules/users/users.controller'
import { forwardToCluster } from '@@/server/services/cluster.service'

export default defineEventHandler(async (event) => {
	const userId = event.context.params?.userId
	const { reqHeaders, query, clusterHeader, allClusters, targetClusters } =
		resolveUserClusterContext(event)

	if (targetClusters.length === 0) {
		return respondNoClustersConfigured(event)
	}

	if (targetClusters.length === 1 && targetClusters[0]) {
		const singleCluster = targetClusters[0]
		const { result: res } = await executeSingleClusterRequestWithAuthRetry(event, {
			item: singleCluster,
			reqHeaders,
			clusterHeader,
			execute: (cluster, headers) =>
				forwardToCluster(cluster, `/api/v1/users/${userId}`, {
					method: 'DELETE',
					headers
				})
		})

		const status = res.status >= 200 && res.status < 300 ? 200 : res.status || 200
		setResponseStatus(event, status)
		return {
			status_code: status,
			success: status >= 200 && status < 300,
			data: res.data?.data || res.data || {}
		}
	}

	const { username, results: clusterResults } = await executeUserActionByUsernameAcrossClusters(
		event,
		{
			allClusters,
			targetClusters,
			userId,
			username: query.username as string | undefined,
			reqHeaders,
			clusterHeader,
			action: (cluster, clusterUserId, headers) =>
				forwardToCluster(cluster, `/api/v1/users/${clusterUserId}`, {
					method: 'DELETE',
					headers
				})
		}
	)

	return respondMultiClusterUserActionResult(event, {
		identifier: username || userId,
		results: clusterResults,
		failureTitle: 'Delete Failed',
		failureDetail: 'Failed to delete user in target clusters.',
		buildSuccess: (successfulResults) => ({
			status: 200,
			data: {
				message: `User ${username || userId} successfully deleted from ${successfulResults.length} cluster(s).`
			}
		})
	})
})

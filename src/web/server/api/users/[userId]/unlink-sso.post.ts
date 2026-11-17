// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { defineEventHandler } from 'h3'
import {
	executeUserActionByUsernameAcrossClusters,
	readUserRequestBody,
	resolveUserClusterContext,
	respondMultiClusterUserActionResult,
	respondNoClustersConfigured
} from '@@/server/modules/users/users.controller'
import { forwardToCluster } from '@@/server/services/cluster.service'

export default defineEventHandler(async (event) => {
	const userId = event.context.params?.userId
	const { reqHeaders, query, clusterHeader, allClusters, targetClusters } =
		resolveUserClusterContext(event)
	const body = await readUserRequestBody(event)

	if (targetClusters.length === 0) {
		return respondNoClustersConfigured(event)
	}

	const { username, results: clusterResults } = await executeUserActionByUsernameAcrossClusters(
		event,
		{
			allClusters,
			targetClusters,
			userId,
			username: body?.username || (query.username as string | undefined),
			reqHeaders,
			clusterHeader,
			action: (cluster, clusterUserId, headers) =>
				forwardToCluster(cluster, `/api/v1/users/${clusterUserId}/unlink-sso`, {
					method: 'POST',
					headers,
					body
				})
		}
	)

	return respondMultiClusterUserActionResult(event, {
		identifier: username || userId,
		results: clusterResults,
		failureTitle: 'Unlink SSO Failed',
		failureDetail: 'Failed to unlink SSO account in target clusters.',
		buildSuccess: (successfulResults) => ({
			status: successfulResults[0]?.status || 200,
			data: {
				message: `SSO successfully unlinked for user ${username || userId} across ${successfulResults.length} cluster(s).`,
				clusters: successfulResults.map((r) => r.cluster.name)
			}
		})
	})
})

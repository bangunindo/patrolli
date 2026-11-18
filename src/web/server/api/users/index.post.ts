// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { defineEventHandler } from 'h3'
import {
	executeClusterRequestsWithAuthRetry,
	executeSingleClusterRequestWithAuthRetry
} from '@@/server/services/auth-refresh.service'
import {
	findClusterUserByUsername,
	formatSingleClusterUserMutationResponse,
	readUserRequestBody,
	resolveUserClusterContext,
	respondMultiClusterUserActionResult,
	respondNoClustersConfigured
} from '@@/server/modules/users/users.controller'
import { forwardToCluster } from '@@/server/services/cluster.service'

export default defineEventHandler(async (event) => {
	const { reqHeaders, clusterHeader, targetClusters } = resolveUserClusterContext(event)
	const body = await readUserRequestBody(event)

	if (body && body.allow_sso === undefined) {
		body.allow_sso = true
	}

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
				forwardToCluster(cluster, '/api/v1/users', {
					method: 'POST',
					headers,
					body
				})
		})

		return formatSingleClusterUserMutationResponse(event, singleCluster, res, 201)
	}

	const username = body?.username

	const { results: clusterResults } = await executeClusterRequestsWithAuthRetry(event, {
		items: targetClusters,
		reqHeaders,
		clusterHeader,
		execute: async (cluster, headers) => {
			try {
				if (username) {
					try {
						const lookup = await findClusterUserByUsername(cluster, username, headers)
						if (lookup.status === 401) {
							return {
								cluster,
								success: false,
								alreadyExisted: false,
								status: 401
							}
						}
						if (lookup.user) {
							return {
								cluster,
								success: true,
								alreadyExisted: true,
								status: 200,
								data: lookup.user
							}
						}
					} catch {
						// Pre-check failed or user not found, proceed to create
					}
				}

				const createRes = await forwardToCluster(cluster, '/api/v1/users', {
					method: 'POST',
					headers,
					body
				})

				return {
					cluster,
					success: createRes.status >= 200 && createRes.status < 300,
					alreadyExisted: false,
					status: createRes.status,
					data: createRes.data?.data || createRes.data || {}
				}
			} catch (err: any) {
				return {
					cluster,
					success: false,
					alreadyExisted: false,
					status: 500,
					error: err.message
				}
			}
		}
	})

	return respondMultiClusterUserActionResult(event, {
		results: clusterResults,
		failureTitle: 'User Creation Failed',
		failureDetail: 'Failed to create user across all specified target clusters.',
		buildSuccess: (successfulResults) => {
			const clusterNames = successfulResults.map((r) => r.cluster.name)
			const primaryData = successfulResults[0]?.data || {}
			return {
				status: 201,
				data: {
					...primaryData,
					clusters: clusterNames,
					cluster_name: clusterNames.join(', ')
				}
			}
		}
	})
})

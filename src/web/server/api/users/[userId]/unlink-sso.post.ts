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

	// Resolve username: either from body, query, or by querying existing user across all clusters
	let username = body?.username || (query.username as string | undefined)
	if (!username && userId) {
		for (const cluster of allClusters) {
			try {
				const res = await forwardToCluster(cluster, `/api/users/${userId}`, {
					method: 'GET',
					headers: reqHeaders
				})
				const uname = res.data?.data?.username || res.data?.username
				if (uname) {
					username = uname
					break
				}
			} catch {
				// try next cluster
			}
		}
	}

	// For each cluster: get user by username, then hit unlink-sso if found, otherwise skip
	const clusterResults = await Promise.all(
		targetClusters.map(async (cluster) => {
			try {
				let clusterUserId = userId

				if (username) {
					const searchRes = await forwardToCluster(cluster, '/api/users', {
						method: 'GET',
						headers: reqHeaders,
						query: { username }
					})
					const clusterUsers =
						searchRes.data?.data?.users ||
						searchRes.data?.users ||
						(Array.isArray(searchRes.data?.data) ? searchRes.data?.data : []) ||
						(Array.isArray(searchRes.data) ? searchRes.data : [])
					const found = clusterUsers.find((u: any) => u.username === username)

					if (found && found.user_id) {
						clusterUserId = found.user_id
					} else {
						// User does not exist in this cluster -> skip
						return { cluster, skipped: true, success: false }
					}
				}

				const unlinkRes = await forwardToCluster(
					cluster,
					`/api/users/${clusterUserId}/unlink-sso`,
					{
						method: 'POST',
						headers: reqHeaders,
						body
					}
				)

				return {
					cluster,
					skipped: false,
					success: unlinkRes.status >= 200 && unlinkRes.status < 300,
					status: unlinkRes.status,
					data: unlinkRes.data?.data || unlinkRes.data
				}
			} catch (err: any) {
				return {
					cluster,
					skipped: false,
					success: false,
					status: 500,
					error: err.message
				}
			}
		})
	)

	const activeResults = clusterResults.filter((r) => !r.skipped)

	if (activeResults.length === 0) {
		setResponseStatus(event, 404)
		return {
			status_code: 404,
			success: false,
			error: {
				title: 'User Not Found',
				detail: `User with username "${username || userId}" was not found in any of the specified target clusters.`
			}
		}
	}

	const successfulResults = activeResults.filter((r) => r.success)

	if (successfulResults.length === 0) {
		const firstFail = activeResults[0]
		setResponseStatus(event, firstFail?.status || 500)
		return {
			status_code: firstFail?.status || 500,
			success: false,
			error: {
				title: 'Unlink SSO Failed',
				detail: 'Failed to unlink SSO account in target clusters.'
			}
		}
	}

	const primarySuccess = successfulResults[0]
	setResponseStatus(event, primarySuccess?.status || 200)
	return {
		status_code: primarySuccess?.status || 200,
		success: true,
		data: {
			message: `SSO successfully unlinked for user ${username || userId} across ${successfulResults.length} cluster(s).`,
			clusters: successfulResults.map((r) => r.cluster.name)
		}
	}
})

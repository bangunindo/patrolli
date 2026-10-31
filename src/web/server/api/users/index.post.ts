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
	const reqHeaders = getRequestHeaders(event)
	const query = getQuery(event)
	let body: any = {}

	try {
		body = await readBody(event)
	} catch {
		body = {}
	}

	if (body && body.allow_sso === undefined) {
		body.allow_sso = true
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

	// 1. Single cluster target: forward POST directly
	if (targetClusters.length === 1 && targetClusters[0]) {
		const singleCluster = targetClusters[0]
		const res = await forwardToCluster(singleCluster, '/api/users', {
			method: 'POST',
			headers: reqHeaders,
			body
		})

		const status = res.status || 201
		setResponseStatus(event, status)

		const clusterName = singleCluster.name
		const clusterUrl = singleCluster.clientPath
		const data = res.data?.data || res.data || {}

		return {
			status_code: status,
			success: status >= 200 && status < 300,
			data: {
				...data,
				clusters: [clusterName],
				cluster_name: clusterName,
				cluster_url: clusterUrl
			}
		}
	}

	// 2. Multi-cluster target: create user in each cluster
	const username = body?.username

	const clusterResults = await Promise.all(
		targetClusters.map(async (cluster) => {
			try {
				// Check if user already exists in this cluster
				if (username) {
					try {
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
						const existingUser = clusterUsers.find((u: any) => u.username === username)

						if (existingUser) {
							return {
								cluster,
								success: true,
								alreadyExisted: true,
								status: 200,
								data: existingUser
							}
						}
					} catch {
						// Pre-check failed or user not found, proceed to create
					}
				}

				// Create user in this cluster
				const createRes = await forwardToCluster(cluster, '/api/users', {
					method: 'POST',
					headers: reqHeaders,
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
		})
	)

	const successfulResults = clusterResults.filter((r) => r.success)

	// If all clusters failed, return error response
	if (successfulResults.length === 0) {
		const firstFail = clusterResults[0]
		setResponseStatus(event, firstFail?.status || 500)
		return {
			status_code: firstFail?.status || 500,
			success: false,
			error: {
				title: 'User Creation Failed',
				detail: 'Failed to create user across all specified target clusters.'
			}
		}
	}

	// Gather clusters that successfully registered/contained the user
	const clusterNames = successfulResults.map((r) => r.cluster.name)
	const primaryData = successfulResults[0]?.data || {}

	setResponseStatus(event, 201)
	return {
		status_code: 201,
		success: true,
		data: {
			...primaryData,
			clusters: clusterNames,
			cluster_name: clusterNames.join(', ')
		}
	}
})

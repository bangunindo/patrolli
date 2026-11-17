// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import {
	getQuery,
	getRequestHeader,
	getRequestHeaders,
	readBody,
	setResponseStatus,
	type H3Event
} from 'h3'
import {
	executeClusterRequestsWithAuthRetry,
	resolveEffectiveRequestHeaders,
	tryTransparentRefreshOn401,
	withBearerAuthorization
} from '@@/server/services/auth-refresh.service'
import {
	forwardToCluster,
	getClustersConfig,
	resolveClusterTargets
} from '@@/server/services/cluster.service'
import type { ICluster } from '@@/shared/types/cluster.type'

export async function readUserRequestBody(event: H3Event): Promise<Record<string, any>> {
	try {
		return (await readBody(event)) || {}
	} catch {
		return {}
	}
}

export function resolveUserClusterContext(event: H3Event) {
	const reqHeaders = getRequestHeaders(event)
	const query = getQuery(event)
	const clusterHeader =
		(reqHeaders['x-cluster-url'] as string | undefined) ||
		(query.cluster as string | undefined) ||
		null
	const allClusters = getClustersConfig(event)
	const targetClusters = resolveClusterTargets(clusterHeader, allClusters)

	return { reqHeaders, query, clusterHeader, allClusters, targetClusters }
}

export function respondNoClustersConfigured(event: H3Event): {
	status_code: number
	success: boolean
	data?: any
	error: { title: string; detail: string }
} {
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

export function formatSingleClusterUserMutationResponse(
	event: H3Event,
	cluster: ICluster,
	res: { status: number; data: any },
	defaultStatus = 200
) {
	const status = res.status || defaultStatus
	setResponseStatus(event, status)
	const data = res.data?.data || res.data || {}
	return {
		status_code: status,
		success: status >= 200 && status < 300,
		data: {
			...data,
			clusters: [cluster.name],
			cluster_name: cluster.name,
			cluster_url: cluster.clientPath
		}
	}
}

export function respondMultiClusterUserActionResult<
	TResult extends { cluster: ICluster; skipped?: boolean; success: boolean; status?: number }
>(
	event: H3Event,
	options: {
		identifier?: string
		results: TResult[]
		failureTitle: string
		failureDetail: string
		buildSuccess: (successfulResults: TResult[]) => {
			status?: number
			data: Record<string, any>
		}
	}
) {
	const activeResults = options.results.filter((r) => !r.skipped)

	if (activeResults.length === 0) {
		setResponseStatus(event, 404)
		return {
			status_code: 404,
			success: false,
			error: {
				title: 'User Not Found',
				detail: `User with username "${options.identifier}" was not found in any of the specified target clusters.`
			}
		}
	}

	const successfulResults = activeResults.filter((r) => r.success)

	if (successfulResults.length === 0) {
		const firstFail = activeResults[0]
		const failStatus = firstFail?.status || 500
		setResponseStatus(event, failStatus)
		return {
			status_code: failStatus,
			success: false,
			error: {
				title: options.failureTitle,
				detail: options.failureDetail
			}
		}
	}

	const { status = 200, data } = options.buildSuccess(successfulResults)
	setResponseStatus(event, status)
	return {
		status_code: status,
		success: true,
		data
	}
}

async function scanClustersForUsername(
	allClusters: ICluster[],
	userId: string,
	headers: Record<string, any>
): Promise<{ username: string | undefined; sawUnauthorized: boolean }> {
	const results = await Promise.all(
		allClusters.map(async (cluster) => {
			try {
				const res = await forwardToCluster(cluster, `/api/v1/users/${userId}`, {
					method: 'GET',
					headers
				})
				if (res?.status === 401) {
					return { username: undefined, isUnauthorized: true }
				}
				const uname = res.data?.data?.username || res.data?.username
				return { username: uname || undefined, isUnauthorized: false }
			} catch {
				return { username: undefined, isUnauthorized: false }
			}
		})
	)

	const username = results.find((r) => Boolean(r.username))?.username
	const sawUnauthorized = results.some((r) => r.isUnauthorized)
	return { username, sawUnauthorized }
}

/**
 * Resolves a username from a userId across clusters, refreshing and retrying once on 401.
 */
export async function resolveUsernameAcrossClustersWithAuthRetry(
	event: H3Event,
	allClusters: ICluster[],
	userId: string,
	headers: Record<string, any>,
	clusterHeader?: string | null
): Promise<{ username: string | undefined; effectiveHeaders: Record<string, any> }> {
	const initial = await scanClustersForUsername(allClusters, userId, headers)
	if (initial.username) {
		return { username: initial.username, effectiveHeaders: headers }
	}

	if (initial.sawUnauthorized && !event.context?._refreshedAccessToken) {
		const newToken = await tryTransparentRefreshOn401(event, clusterHeader)
		if (newToken) {
			const updatedHeaders = withBearerAuthorization(headers, newToken)
			const retried = await scanClustersForUsername(allClusters, userId, updatedHeaders)
			return { username: retried.username, effectiveHeaders: updatedHeaders }
		}
	}

	return { username: undefined, effectiveHeaders: headers }
}

/**
 * Looks up a user by exact username on a single cluster, normalizing response shapes.
 */
export async function findClusterUserByUsername(
	cluster: ICluster,
	username: string,
	headers: Record<string, any>
): Promise<{ status: number; user: any | null }> {
	const searchRes = await forwardToCluster(cluster, '/api/v1/users', {
		method: 'GET',
		headers,
		query: { username }
	})
	if (searchRes.status === 401) {
		return { status: 401, user: null }
	}
	const clusterUsers = Array.isArray(searchRes.data?.data?.users)
		? searchRes.data.data.users
		: Array.isArray(searchRes.data?.users)
			? searchRes.data.users
			: Array.isArray(searchRes.data?.data)
				? searchRes.data.data
				: Array.isArray(searchRes.data)
					? searchRes.data
					: []
	const found = clusterUsers.find((u: any) => u.username === username) ?? null
	return { status: searchRes.status, user: found }
}

/**
 * Resolves a user's username across clusters if needed, then locates the matching user per target cluster
 * and executes the given action with pre-emptive and reactive 401 token refresh.
 */
export async function executeUserActionByUsernameAcrossClusters(
	event: H3Event,
	options: {
		allClusters: ICluster[]
		targetClusters: ICluster[]
		userId?: string
		username?: string
		reqHeaders: Record<string, any>
		clusterHeader?: string | null
		action: (
			cluster: ICluster,
			clusterUserId: string | undefined,
			headers: Record<string, any>
		) => Promise<{ status: number; data: any }>
	}
) {
	let effectiveHeaders = await resolveEffectiveRequestHeaders(
		event,
		options.reqHeaders,
		options.clusterHeader
	)

	let username = options.username
	if (!username && options.userId) {
		const resolved = await resolveUsernameAcrossClustersWithAuthRetry(
			event,
			options.allClusters,
			options.userId,
			effectiveHeaders,
			options.clusterHeader
		)
		username = resolved.username
		effectiveHeaders = resolved.effectiveHeaders
	}

	const { results } = await executeClusterRequestsWithAuthRetry(event, {
		items: options.targetClusters,
		reqHeaders: effectiveHeaders,
		clusterHeader: options.clusterHeader,
		execute: async (cluster, headers) => {
			try {
				let clusterUserId = options.userId

				if (username) {
					const lookup = await findClusterUserByUsername(cluster, username, headers)
					if (lookup.status === 401) {
						return {
							cluster,
							skipped: false,
							success: false,
							status: 401,
							data: undefined
						}
					}
					if (lookup.user?.user_id) {
						clusterUserId = lookup.user.user_id
					} else {
						return {
							cluster,
							skipped: true,
							success: false,
							status: 200,
							data: undefined
						}
					}
				}

				const res = await options.action(cluster, clusterUserId, headers)
				return {
					cluster,
					skipped: false,
					success: res.status >= 200 && res.status < 300,
					status: res.status,
					data: res.data?.data || res.data
				}
			} catch (err: any) {
				return {
					cluster,
					skipped: false,
					success: false,
					status: 500,
					data: undefined,
					error: err.message
				}
			}
		}
	})

	return { username, results }
}

/**
 * Aggregates user summary counts across all targeted clusters, falling back to
 * `/api/v1/users?limit=1000` when the cluster does not support `/api/v1/users/summary`.
 */
export async function handleUsersSummary(event: H3Event) {
	const allClusters = getClustersConfig(event)
	const clusterHeader = getRequestHeader(event, 'x-cluster-url')
	const targetClusters = resolveClusterTargets(clusterHeader, allClusters)
	const reqHeaders = getRequestHeaders(event)

	let totalUser = 0
	let totalActiveUser = 0
	let totalDisabledUser = 0
	let totalAllowSso = 0
	const userCluster: Record<string, number> = {}

	if (targetClusters.length > 0) {
		const { results } = await executeClusterRequestsWithAuthRetry(event, {
			items: targetClusters,
			reqHeaders,
			clusterHeader,
			execute: async (cluster, headers) => {
				let data: { total?: number; allow_sso?: number; disabled?: number } | undefined
				try {
					const res = await forwardToCluster(cluster, '/api/v1/users/summary', {
						method: 'GET',
						headers
					})
					if (res?.status === 401) {
						return { cluster, status: 401, data: undefined }
					}
					if ((res?.status === undefined || res.status < 400) && res?.data?.data) {
						data = res.data.data
					}
				} catch {
					// Backend summary failed or not supported, fallback below
				}

				if (!data) {
					try {
						const usersRes = await forwardToCluster(
							cluster,
							'/api/v1/users?limit=1000',
							{
								method: 'GET',
								headers
							}
						)
						if (usersRes?.status === 401) {
							return { cluster, status: 401, data: undefined }
						}
						if (
							(usersRes?.status === undefined || usersRes.status < 400) &&
							usersRes?.data?.data
						) {
							const users = usersRes.data.data.users || []
							const countTotal = Number(usersRes.data.data.total ?? users.length)
							const countAllowSso = users.filter((u: any) =>
								Boolean(u.allow_sso)
							).length
							const countDisabled = users.filter(
								(u: any) => u.adm_status === 0
							).length
							data = {
								total: countTotal,
								allow_sso: countAllowSso,
								disabled: countDisabled
							}
						}
					} catch (err: any) {
						console.error(
							`[summary] Fallback /api/users failed for ${cluster.name}:`,
							err?.message
						)
					}
				}

				return { cluster, status: 200, data }
			}
		})

		for (const { cluster, data } of results) {
			const total = Number(data?.total ?? 0)
			const disabled = Number(data?.disabled ?? 0)
			const allowSso = Number(data?.allow_sso ?? 0)

			const validTotal = Number.isFinite(total) ? total : 0
			const validDisabled = Number.isFinite(disabled) ? disabled : 0
			const validAllowSso = Number.isFinite(allowSso) ? allowSso : 0

			userCluster[cluster.name] = validTotal
			totalUser += validTotal
			totalDisabledUser += validDisabled
			totalActiveUser += Math.max(0, validTotal - validDisabled)
			totalAllowSso += validAllowSso
		}
	}

	return {
		status_code: 200,
		success: true,
		data: {
			total_user: totalUser,
			total_active_user: totalActiveUser,
			total_disabled_user: totalDisabledUser,
			total_allow_sso: totalAllowSso,
			user_cluster: userCluster
		}
	}
}

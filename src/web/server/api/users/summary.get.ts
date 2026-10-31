// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { defineEventHandler, getRequestHeader, getRequestHeaders } from 'h3'
import {
	forwardToCluster,
	getClustersConfig,
	resolveClusterTargets
} from '@@/server/services/cluster.service'

export default defineEventHandler(async (event) => {
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
		await Promise.all(
			targetClusters.map(async (cluster) => {
				let data: { total?: number; allow_sso?: number; disabled?: number } | undefined
				try {
					const res = await forwardToCluster(cluster, '/api/users/summary', {
						method: 'GET',
						headers: reqHeaders
					})
					if ((res.status === undefined || res.status < 400) && res.data?.data) {
						data = res.data.data
					}
				} catch {
					// Backend summary failed or not supported, fallback below
				}

				if (!data) {
					try {
						const usersRes = await forwardToCluster(cluster, '/api/users?limit=1000', {
							method: 'GET',
							headers: reqHeaders
						})
						if (
							(usersRes.status === undefined || usersRes.status < 400) &&
							usersRes.data?.data
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
			})
		)
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
})

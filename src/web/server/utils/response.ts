// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { type H3Event, setResponseHeaders, setResponseStatus } from 'h3'
import type { ICluster } from '@@/shared/types/cluster.type'

export interface ClusterResponseItem {
	cluster: ICluster
	data: any
	status: number
}

/**
 * Merges responses from single or multiple cluster nodes into a unified response
 */
export function mergeDynamicResponses(responses: ClusterResponseItem[]): any {
	if (responses.length === 0) {
		return {
			status_code: 200,
			success: true,
			data: {}
		}
	}

	// 1. Single cluster response
	if (responses.length === 1) {
		const single = responses[0]
		if (single.status >= 400 || single.data?.success === false) {
			return {
				status_code: single.data?.status_code ?? single.status ?? 400,
				success: false,
				error: single.data?.error ?? {
					title: 'Request Failed',
					detail: single.data?.message || 'Operation failed'
				},
				data: single.data?.data ?? single.data ?? {}
			}
		}

		const clusterClientPath = single.cluster.clientPath
		const clusterName = single.cluster.name
		const singleData = single.data?.data !== undefined ? single.data.data : single.data

		if (singleData && typeof singleData === 'object' && !Array.isArray(singleData)) {
			const enriched: Record<string, any> = { ...singleData }
			Object.entries(singleData).forEach(([key, value]) => {
				if (Array.isArray(value)) {
					enriched[key] = value.map((item) =>
						typeof item === 'object' && item !== null
							? {
									...item,
									cluster_url: clusterClientPath,
									cluster_name: clusterName
								}
							: item
					)
				}
			})
			if (enriched.cluster_url === undefined) enriched.cluster_url = clusterClientPath
			if (enriched.cluster_name === undefined) enriched.cluster_name = clusterName

			return {
				status_code: single.data?.status_code ?? single.status ?? 200,
				success: true,
				data: enriched
			}
		}

		return {
			status_code: single.data?.status_code ?? single.status ?? 200,
			success: true,
			data: single.data
		}
	}

	// 2. Multi-cluster response: filter for successful responses
	const successfulResponses = responses.filter(
		(r) => r.status >= 200 && r.status < 300 && r.data?.success !== false
	)

	// If all clusters failed, return the first failure
	if (successfulResponses.length === 0) {
		const firstFail = responses[0]
		return {
			status_code: firstFail?.data?.status_code ?? firstFail?.status ?? 500,
			success: false,
			error: firstFail?.data?.error ?? {
				title: 'Clusters Unavailable',
				detail: 'Unable to retrieve data from any cluster'
			},
			data: {}
		}
	}

	const mergedData: Record<string, any> = {}
	let totalRecords = 0
	let hasTotalOrCountOrArray = false

	successfulResponses.forEach((res) => {
		const clusterClientPath = res.cluster.clientPath
		const clusterName = res.cluster.name
		const responseBody = res.data
		const responseData = responseBody?.data !== undefined ? responseBody.data : responseBody

		if (typeof responseData?.total === 'number') {
			totalRecords += responseData.total
			hasTotalOrCountOrArray = true
		} else if (typeof responseData?.count === 'number') {
			totalRecords += responseData.count
			hasTotalOrCountOrArray = true
		}

		if (responseData && typeof responseData === 'object' && !Array.isArray(responseData)) {
			Object.entries(responseData).forEach(([key, value]) => {
				if (Array.isArray(value)) {
					if (
						typeof responseData?.total !== 'number' &&
						typeof responseData?.count !== 'number'
					) {
						totalRecords += value.length
					}
					if (!mergedData[key]) {
						mergedData[key] = []
					}
					const itemsWithCluster = value.map((item) =>
						typeof item === 'object' && item !== null
							? {
									...item,
									cluster_url: clusterClientPath,
									cluster_name: clusterName
								}
							: item
					)
					mergedData[key].push(...itemsWithCluster)
				} else if (key !== 'total' && key !== 'count') {
					if (mergedData[key] === undefined) {
						mergedData[key] = value
					}
				}
			})

			if (!hasTotalOrCountOrArray) {
				mergedData.cluster_url = clusterClientPath
				mergedData.cluster_name = clusterName
			}
		}
	})

	if (hasTotalOrCountOrArray) {
		mergedData.total = totalRecords
	}

	return {
		status_code: 200,
		success: true,
		data: mergedData
	}
}

/**
 * Handles single cluster response forwarding headers and status
 */
export function handleSingleClusterResponse(
	event: H3Event,
	target: ICluster,
	res: { status: number; headers: Headers; data: any }
): any {
	const clusterSetCookie = (res.headers as any).getSetCookie
		? (res.headers as any).getSetCookie()
		: res.headers.get('set-cookie')
	if (clusterSetCookie) {
		setResponseHeaders(event, { 'set-cookie': clusterSetCookie })
	}

	if (res.data && typeof res.data === 'object') {
		const merged = mergeDynamicResponses([
			{ cluster: target, data: res.data, status: res.status }
		])
		setResponseStatus(event, merged.status_code || res.status)
		return merged
	}

	setResponseStatus(event, res.status)
	return res.data
}

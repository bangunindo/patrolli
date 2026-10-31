// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { ICluster } from '@@/shared/types/cluster.type'
import { forwardToCluster } from '@@/server/services/cluster.service'

export interface PaginationDistribution {
	offset: number
	limit: number
}

export interface PaginatedClusterResponse {
	cluster: ICluster
	plan: {
		localOffset: number
		localLimit: number
	}
	data: any
	status: number
}

export interface RebalanceOptions {
	mergedResult: any
	initialResponses: PaginatedClusterResponse[]
	fullApiPath: string
	method: string
	reqHeaders: Record<string, any>
	query: Record<string, any>
	offset: number
	limit: number
}

/**
 * Distributes global pagination offset and limit evenly across multiple cluster nodes.
 */
export function distributePagination(
	globalOffset: number,
	globalLimit: number,
	nodeCount: number,
	nodeIndex: number
): PaginationDistribution {
	if (nodeCount <= 1) {
		return { offset: globalOffset, limit: globalLimit }
	}

	const baseLimit = Math.floor(globalLimit / nodeCount)
	const extraLimit = globalLimit % nodeCount
	const localLimit = nodeIndex < extraLimit ? baseLimit + 1 : baseLimit

	const pageIndex = Math.floor(globalOffset / globalLimit)
	const remainderOffset = globalOffset % globalLimit
	const localOffsetExtra =
		nodeIndex < remainderOffset % nodeCount
			? Math.floor(remainderOffset / nodeCount) + 1
			: Math.floor(remainderOffset / nodeCount)
	const localOffset = pageIndex * localLimit + localOffsetExtra

	return { offset: localOffset, limit: localLimit }
}

/**
 * Helper to count the maximum number of items currently in the merged response arrays.
 */
function countFetchedItems(mergedData: any): number {
	if (!mergedData) return 0
	let maxCount = 0
	for (const value of Object.values(mergedData)) {
		if (Array.isArray(value)) {
			maxCount = Math.max(maxCount, value.length)
		}
	}
	return maxCount
}

/**
 * Helper to unwrap response payload regardless of whether it is wrapped in .data or not.
 */
function unwrapResponseData(responsePayload: any): any {
	if (responsePayload?.data !== undefined) {
		return responsePayload.data
	}
	return responsePayload
}

/**
 * Helper to find the primary array of items inside a cluster response data object.
 */
function findItemsList(responseData: any): any[] {
	if (!responseData || typeof responseData !== 'object') return []
	for (const value of Object.values(responseData)) {
		if (Array.isArray(value)) {
			return value
		}
	}
	return []
}

/**
 * Appends extra fetched items into matching array keys of the merged result,
 * tagging each item with its source cluster metadata.
 */
function appendExtraItemsToMergedResult(
	mergedData: Record<string, any>,
	extraData: Record<string, any>,
	cluster: ICluster
): number {
	let itemsAdded = 0

	for (const [key, extraList] of Object.entries(extraData)) {
		if (Array.isArray(extraList) && Array.isArray(mergedData[key])) {
			const enrichedItems = extraList.map((item) => {
				if (typeof item === 'object' && item !== null) {
					return {
						...item,
						cluster_url: cluster.clientPath,
						cluster_name: cluster.name
					}
				}
				return item
			})

			mergedData[key].push(...enrichedItems)
			itemsAdded = Math.max(itemsAdded, extraList.length)
		}
	}

	return itemsAdded
}

/**
 * Rebalances pagination results when some clusters returned fewer items than their allocated quota,
 * while other clusters still have remaining items within the global available range.
 */
export async function rebalancePaginationResponses(options: RebalanceOptions): Promise<void> {
	const {
		mergedResult,
		initialResponses,
		fullApiPath,
		method,
		reqHeaders,
		query,
		offset,
		limit: targetLimit
	} = options

	if (!mergedResult?.data) return

	// 1. Calculate how many items we currently have vs how many are still missing
	const fetchedCount = countFetchedItems(mergedResult.data)
	const totalGloballyAvailable = mergedResult.data.total ?? 0
	const remainingGlobalItems = Math.max(0, totalGloballyAvailable - offset)

	// No rebalance needed if we already filled the quota or no more items exist globally
	if (fetchedCount >= targetLimit || fetchedCount >= remainingGlobalItems) {
		return
	}

	let missingCount = Math.min(targetLimit - fetchedCount, remainingGlobalItems - fetchedCount)

	// 2. Fetch missing items from clusters that still have remaining data
	for (const response of initialResponses) {
		if (missingCount <= 0) break

		const responseData = unwrapResponseData(response.data)
		const nodeTotal = responseData?.total ?? responseData?.count ?? 0
		const initialItems = findItemsList(responseData)

		const nextOffset = response.plan.localOffset + initialItems.length
		const availableInNode = nodeTotal - nextOffset

		if (availableInNode <= 0) continue

		const fetchCount = Math.min(missingCount, availableInNode)
		const extraQuery = {
			...query,
			offset: nextOffset,
			limit: fetchCount
		}

		try {
			const extraRes = await forwardToCluster(response.cluster, fullApiPath, {
				method,
				headers: reqHeaders,
				query: extraQuery
			})

			if (extraRes?.data) {
				const extraData = unwrapResponseData(extraRes.data)
				const addedCount = appendExtraItemsToMergedResult(
					mergedResult.data,
					extraData,
					response.cluster
				)
				missingCount -= addedCount
			}
		} catch {
			// Ignore individual cluster rebalance failures
		}
	}
}

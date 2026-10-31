// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { ICluster } from '@@/shared/types/cluster.type'
import type { IClusterClientItem } from './clusters.type'

/**
 * Formats internal cluster configurations into client-consumable list items
 */
export function formatClusterList(clusters: ICluster[]): IClusterClientItem[] {
	return clusters.map((c) => ({
		key: c.key || c.name,
		name: c.name,
		url: c.clientPath || `/${c.name.replace(/^\//, '')}`,
		clientPath: c.clientPath || `/${c.name.replace(/^\//, '')}`,
		label: c.name,
		value: c.clientPath || `/${c.name.replace(/^\//, '')}`
	}))
}

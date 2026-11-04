// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import clustersApi, { formatClusterOptions } from '@/app/api/clusters.api'
import { useAppStore } from '@/stores/app.store'

export default defineNuxtPlugin(async () => {
	try {
		const resp = await clustersApi.getClusters()
		if (resp) {
			const rawItems = Array.isArray(resp?.data) ? resp.data : Array.isArray(resp) ? resp : []
			const formatted = formatClusterOptions(rawItems)

			window.__APP_CONFIG__ = {
				...window.__APP_CONFIG__,
				API_URLS: formatted.map((c) => c.name)
			}

			if (formatted.length > 0) {
				try {
					const appStore = useAppStore()
					appStore.setAvailableClusters(formatted)
				} catch {
					// Store not ready yet
				}
			}
		}
	} catch (error) {
		console.error('Failed to load runtime configuration (/api/clusters):', error)
	}
})

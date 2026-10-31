// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { useAppStore } from '@/stores/app.store'

export default defineNuxtPlugin(async () => {
	try {
		const response = await fetch('/api/clusters')
		if (response.ok) {
			const resp = await response.json()
			const clusterList = Array.isArray(resp?.data)
				? resp.data.map((c: any) =>
						typeof c === 'string' ? c : c.name || c.key || c.value
					)
				: Array.isArray(resp)
					? resp
					: []

			window.__APP_CONFIG__ = {
				...window.__APP_CONFIG__,
				API_URLS: clusterList
			}

			try {
				const formatted = Array.isArray(resp?.data)
					? resp.data.map((item: any) => ({
							alias: item.key || item.name || item.value,
							name: item.name || item.key || item.value,
							label: item.label || item.name || item.key || item.value,
							value: item.value || item.clientPath || item.url || item.name
						}))
					: []
				if (formatted.length > 0) {
					const appStore = useAppStore()
					appStore.setAvailableClusters(formatted)
				}
			} catch {
				// Store not ready yet
			}
		}
	} catch (error) {
		console.error('Failed to load runtime configuration (/api/clusters):', error)
	}
})

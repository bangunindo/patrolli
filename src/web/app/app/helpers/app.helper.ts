// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { useAppStore } from '@/stores/app.store'
import { getActivePinia } from 'pinia'
import cookieHelper from '@/app/helpers/cookie.helper'

export default {
	parseClustersFromSource() {
		if (typeof window !== 'undefined') {
			try {
				const cookieVal = cookieHelper.get('user_clusters')
				if (cookieVal) {
					const parsed = JSON.parse(cookieVal)
					if (Array.isArray(parsed) && parsed.length > 0) {
						return parsed.map((item: any) => ({
							alias: item.key || item.name || item.value,
							name: item.name || item.key || item.value,
							label: item.label || item.name || item.key || item.value,
							value: item.value || item.clientPath || item.url || item.name
						}))
					}
				}
			} catch (error) {
				console.error('Failed to parse user_clusters cookie:', error)
			}
		}

		const config = typeof window !== 'undefined' ? window.__APP_CONFIG__ : undefined
		const clusterKeys = config?.API_URLS

		if (!clusterKeys || !Array.isArray(clusterKeys) || clusterKeys.length === 0) {
			const key = 'node-default'
			return [
				{
					alias: key,
					name: key,
					label: key,
					value: key
				}
			]
		}

		try {
			return clusterKeys.map((key) => ({
				alias: key,
				name: key,
				label: key,
				value: key
			}))
		} catch (error) {
			console.error('Failed to parse cluster configuration:', error)
			return []
		}
	},

	getBaseURLs() {
		if (getActivePinia()) {
			try {
				const store = useAppStore()
				if (store?.availableClusters && store.availableClusters.length > 0) {
					return store.availableClusters
				}
			} catch {
				// Pinia store not initialized yet
			}
		}

		return this.parseClustersFromSource()
	},

	getDefaultClusterValue() {
		const clusters = this.getBaseURLs()
		if (clusters.length === 1 && clusters[0]?.value) {
			return clusters[0].value
		}
		let globalClusterURL: string | null | undefined = undefined
		if (getActivePinia()) {
			try {
				const store = useAppStore()
				globalClusterURL = store?.activeClusterURL
			} catch {
				// Pinia store not initialized yet
			}
		}
		const defaultURLs = globalClusterURL ?? clusters[0]?.value

		return defaultURLs
	},

	getClusterName(clusterURL?: string) {
		if (!clusterURL) return ''
		const clusters = this.getBaseURLs()
		const cleanKey = clusterURL.replace(/^\//, '')
		const matched = clusters.find(
			(c) =>
				c.value === clusterURL ||
				c.value === cleanKey ||
				c.name === clusterURL ||
				c.name === cleanKey ||
				c.alias === clusterURL ||
				c.alias === cleanKey
		)
		return matched?.name ?? matched?.label ?? cleanKey
	},

	getClusterValue(clusterName?: string) {
		if (!clusterName) return ''
		const clusters = this.getBaseURLs()
		const cleanKey = clusterName.replace(/^\//, '')
		const matched = clusters.find(
			(c) =>
				c.name === clusterName ||
				c.name === cleanKey ||
				c.alias === clusterName ||
				c.alias === cleanKey ||
				c.label === clusterName ||
				c.label === cleanKey ||
				c.value === clusterName ||
				c.value === cleanKey
		)
		return matched?.value ?? clusterName
	}
}

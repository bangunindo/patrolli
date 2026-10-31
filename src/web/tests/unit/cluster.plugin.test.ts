// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
;(globalThis as any).defineNuxtPlugin = (fn: any) => fn

describe('Cluster Client Plugin', () => {
	beforeEach(() => {
		setActivePinia(createPinia())
		window.__APP_CONFIG__ = {}
		vi.restoreAllMocks()
	})

	it('should fetch cluster list and store in window.__APP_CONFIG__.API_URLS', async () => {
		vi.spyOn(globalThis, 'fetch').mockResolvedValue({
			ok: true,
			json: async () => ({
				status_code: 200,
				success: true,
				data: [
					{
						url: 'node-default',
						name: 'node-default',
						label: 'node-default',
						value: '/node-default'
					},
					{
						url: 'patrolli-cluster-01',
						name: 'patrolli-cluster-01',
						label: 'patrolli-cluster-01',
						value: '/patrolli-cluster-01'
					}
				]
			})
		} as any)

		const { default: clusterPlugin } = await import('@/plugins/app-config.client')
		const nuxtAppMock = {} as any
		await (clusterPlugin as any)(nuxtAppMock)

		expect(window.__APP_CONFIG__.API_URLS).toEqual(['node-default', 'patrolli-cluster-01'])
	})
})

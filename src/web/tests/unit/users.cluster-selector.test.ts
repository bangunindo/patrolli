// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import ClusterSelector from '@/components/users/ClusterSelector.vue'
import { useAuthStore } from '@/stores/auth.store'
import { useAppStore } from '@/stores/app.store'
import appHelper from '@/app/helpers/app.helper'

describe('ClusterSelector Component - Single vs Multi-Cluster Login', () => {
	const globalStubs = {
		'b-icon': {
			template: '<span class="b-icon-stub" />'
		}
	}

	beforeEach(() => {
		setActivePinia(createPinia())
		vi.spyOn(appHelper, 'getBaseURLs').mockReturnValue([
			{
				alias: 'patrolli1',
				name: 'patrolli1',
				label: 'Cluster 1 (patrolli1)',
				value: 'patrolli1'
			},
			{
				alias: 'patrolli2',
				name: 'patrolli2',
				label: 'Cluster 2 (patrolli2)',
				value: 'patrolli2'
			},
			{
				alias: 'patrolli3',
				name: 'patrolli3',
				label: 'Cluster 3 (patrolli3)',
				value: 'patrolli3'
			}
		])
	})

	it('should ONLY display the user’s single cluster when logged in as single cluster (local login)', async () => {
		const authStore = useAuthStore()
		const appStore = useAppStore()

		// Simulate single cluster login (local provider)
		authStore.authProvider = 'local'
		expect(authStore.isClusterReadonly).toBe(true)

		// User is logged into patrolli2
		appStore.activeClusterURL = 'patrolli2'

		const wrapper = mount(ClusterSelector, {
			props: {
				modelValue: []
			},
			global: {
				stubs: globalStubs
			}
		})

		await flushPromises()

		// Open dropdown
		await wrapper.find('button').trigger('click')
		await flushPromises()

		// Verify available clusters rendered
		const checkboxes = wrapper.findAll('input[type="checkbox"]')
		// "Select All Clusters" should be hidden because only 1 cluster is available
		expect(wrapper.text()).not.toContain('Select All Clusters')

		// Only 1 cluster checkbox should exist
		expect(checkboxes.length).toBe(1)
		expect(wrapper.text()).toContain('Cluster 2 (patrolli2)')
		expect(wrapper.text()).not.toContain('Cluster 1 (patrolli1)')
		expect(wrapper.text()).not.toContain('Cluster 3 (patrolli3)')

		// Verify modelValue was auto-selected to patrolli2
		const emitted = wrapper.emitted('update:modelValue')
		expect(emitted).toBeTruthy()
		expect(emitted?.[0]?.[0]).toEqual(['patrolli2'])
	})

	it('should display ALL clusters when logged in as multi-cluster (SSO/Keycloak)', async () => {
		const authStore = useAuthStore()
		const appStore = useAppStore()

		// Simulate multi-cluster SSO login
		authStore.authProvider = 'keycloak'
		expect(authStore.isClusterReadonly).toBe(false)
		appStore.activeClusterURL = null

		const wrapper = mount(ClusterSelector, {
			props: {
				modelValue: ['patrolli1']
			},
			global: {
				stubs: globalStubs
			}
		})

		await flushPromises()

		// Open dropdown
		await wrapper.find('button').trigger('click')
		await flushPromises()

		// "Select All Clusters" should be visible
		expect(wrapper.text()).toContain('Select All Clusters')

		// All 3 clusters should be displayed
		expect(wrapper.text()).toContain('Cluster 1 (patrolli1)')
		expect(wrapper.text()).toContain('Cluster 2 (patrolli2)')
		expect(wrapper.text()).toContain('Cluster 3 (patrolli3)')

		// Total 4 checkboxes (1 Select All + 3 clusters)
		const checkboxes = wrapper.findAll('input[type="checkbox"]')
		expect(checkboxes.length).toBe(4)
	})
})

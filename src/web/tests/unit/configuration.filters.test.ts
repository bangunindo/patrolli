// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, it, expect, beforeEach, vi } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { useEndpointsStore } from '@/stores/endpoints.store'
import { useListenerStore } from '@/stores/listeners.store'
import { useItemsStore } from '@/stores/items.store'
import { useSinksStore } from '@/stores/sinks.store'

vi.stubGlobal('definePageMeta', vi.fn())

vi.mock('vue-router', () => ({
	useRoute: () => ({
		query: {},
		params: {},
		path: '/configuration/item'
	}),
	useRouter: () => ({
		push: vi.fn(),
		replace: vi.fn()
	})
}))

import EndpointIndex from '@/pages/configuration/endpoint/index.vue'
import ListenerIndex from '@/pages/configuration/listener/index.vue'
import ItemIndex from '@/pages/configuration/item/index.vue'
import SinkIndex from '@/pages/configuration/sink/index.vue'

vi.mock('@/app/services/http.service', () => ({
	httpService: {
		get: vi.fn().mockResolvedValue({
			status_code: 200,
			success: true,
			data: {
				endpoints: [],
				listeners: [],
				items: [],
				processors: [],
				sinks: [],
				hosts: [],
				total: 0
			}
		}),
		post: vi.fn(),
		put: vi.fn(),
		delete: vi.fn()
	}
}))

vi.mock('@/app/api/hosts.api', () => ({
	default: {
		getHosts: vi.fn().mockResolvedValue({
			status_code: 200,
			success: true,
			data: {
				hosts: [
					{ host_id: 'host-1', name: 'Host Alpha' },
					{ host_id: 'host-2', name: 'Host Beta' }
				]
			}
		})
	}
}))

const globalStubs = {
	AppLayoutPageContainer: { template: '<div><slot /></div>' },
	AppLayoutPageHeader: { template: '<div><slot /><slot name="actions" /></div>' },
	UiButton: { template: '<button><slot /></button>' },
	UiIcon: { template: '<span class="icon-stub" />' },
	UiDropdownMenu: { template: '<div class="dropdown"><slot /></div>' },
	UiDropdownMenuTrigger: { template: '<div><slot /></div>' },
	UiDropdownMenuContent: { template: '<div class="dropdown-content"><slot /></div>' },
	'b-container': { template: '<div><slot /></div>' },
	'b-flex': { template: '<div><slot /></div>' },
	'b-flex-item': { template: '<div><slot /></div>' },
	'b-heading': { template: '<h1><slot /></h1>' },
	'b-text': { template: '<span><slot /></span>' },
	'b-button': { template: '<button><slot /></button>' },
	'b-alert': { template: '<div><slot /></div>' },
	'b-icon': { template: '<span class="icon-stub" />' },
	'endpoint-table': { template: '<div class="endpoint-table-stub" />' },
	'listener-table': { template: '<div class="listener-table-stub" />' },
	'items-table': { template: '<div class="items-table-stub" />' },
	'processor-table': { template: '<div class="processor-table-stub" />' },
	'sink-table': { template: '<div class="sink-table-stub" />' },
	'config-drawer': { template: '<div class="drawer-stub" />' },
	stats: { template: '<div class="stats-stub" />' },
	'b-dropdown': { template: '<div class="b-dropdown"><slot /></div>' },
	'b-dropdown-toggle': { template: '<div><slot /></div>' },
	'b-dropdown-content': { template: '<div class="dropdown-content"><slot /></div>' },
	teleport: true
}

describe('Configuration Filters with AppInputSelect', () => {
	beforeEach(() => {
		setActivePinia(createPinia())
		vi.clearAllMocks()
	})

	describe('Endpoints Filter', () => {
		it('should render AppInputSelect for Type and Host', async () => {
			const wrapper = mount(EndpointIndex, { global: { stubs: globalStubs } })
			await flushPromises()

			const selects = wrapper.findAllComponents({ name: 'AppInputSelect' })
			expect(selects.length).toBe(2)
			expect(selects[0].props('label')).toBe('Type')
			expect(selects[0].props('clearable')).toBe(true)
			expect(selects[1].props('label')).toBe('Host')
			expect(selects[1].props('clearable')).toBe(true)
		})

		it('should update endpointsStore.filter and reset on clear', async () => {
			const store = useEndpointsStore()
			const wrapper = mount(EndpointIndex, { global: { stubs: globalStubs } })
			await flushPromises()

			const selects = wrapper.findAllComponents({ name: 'AppInputSelect' })
			await selects[0].vm.$emit('update:modelValue', 'snmp_v2c')
			await flushPromises()
			expect(store.filter.types).toEqual(['snmp_v2c'])

			await selects[0].vm.$emit('update:modelValue', null)
			await flushPromises()
			expect(store.filter.types).toBeUndefined()
		})
	})

	describe('Listeners Filter', () => {
		it('should render AppInputSelect for Type', async () => {
			const wrapper = mount(ListenerIndex, { global: { stubs: globalStubs } })
			await flushPromises()

			const selects = wrapper.findAllComponents({ name: 'AppInputSelect' })
			expect(selects.length).toBe(1)
			expect(selects[0].props('label')).toBe('Type')
			expect(selects[0].props('placeholder')).toBe('All Types')
			expect(selects[0].props('clearable')).toBe(true)
		})

		it('should update listenersStore.filter and reset on clear', async () => {
			const store = useListenerStore()
			const wrapper = mount(ListenerIndex, { global: { stubs: globalStubs } })
			await flushPromises()

			const selects = wrapper.findAllComponents({ name: 'AppInputSelect' })
			await selects[0].vm.$emit('update:modelValue', 'snmp_trap')
			await flushPromises()
			expect(store.filter.types).toEqual(['snmp_trap'])

			await selects[0].vm.$emit('update:modelValue', null)
			await flushPromises()
			expect(store.filter.types).toBeUndefined()
		})
	})

	describe('Items Filter', () => {
		it('should render AppInputSelect for Type, Endpoint, and Listener', async () => {
			const wrapper = mount(ItemIndex, { global: { stubs: globalStubs } })
			await flushPromises()

			const selects = wrapper.findAllComponents({ name: 'AppInputSelect' })
			expect(selects.length).toBe(3)
			expect(selects[0].props('label')).toBe('Type')
			expect(selects[1].props('label')).toBe('Endpoint')
			expect(selects[2].props('label')).toBe('Listener')
		})

		it('should update itemsStore.filter and reset on clear', async () => {
			const store = useItemsStore()
			const wrapper = mount(ItemIndex, { global: { stubs: globalStubs } })
			await flushPromises()

			const selects = wrapper.findAllComponents({ name: 'AppInputSelect' })
			await selects[0].vm.$emit('update:modelValue', 'snmp')
			await selects[1].vm.$emit('update:modelValue', 'ep-123')
			await flushPromises()

			expect(store.filter.types).toEqual(['snmp'])
			expect(store.filter.endpoint_ids).toEqual(['ep-123'])

			await selects[0].vm.$emit('update:modelValue', null)
			await flushPromises()
			expect(store.filter.types).toBeUndefined()
		})
	})

	describe('Sinks Filter', () => {
		it('should render AppInputSelect for Type', async () => {
			const wrapper = mount(SinkIndex, { global: { stubs: globalStubs } })
			await flushPromises()

			const selects = wrapper.findAllComponents({ name: 'AppInputSelect' })
			expect(selects.length).toBe(1)
			expect(selects[0].props('label')).toBe('Type')
			expect(selects[0].props('clearable')).toBe(true)
		})

		it('should update sinksStore.filter and reset on clear', async () => {
			const store = useSinksStore()
			const wrapper = mount(SinkIndex, { global: { stubs: globalStubs } })
			await flushPromises()

			const selects = wrapper.findAllComponents({ name: 'AppInputSelect' })
			await selects[0].vm.$emit('update:modelValue', 'kafka')
			await flushPromises()
			expect(store.filter.types).toEqual(['kafka'])

			await selects[0].vm.$emit('update:modelValue', null)
			await flushPromises()
			expect(store.filter.types).toBeUndefined()
		})
	})
})

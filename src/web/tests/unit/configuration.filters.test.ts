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
	'endpoint-table': { template: '<div class="endpoint-table-stub" />' },
	'listener-table': { template: '<div class="listener-table-stub" />' },
	'items-table': { template: '<div class="items-table-stub" />' },
	'sink-table': { template: '<div class="sink-table-stub" />' },
	'config-drawer': { template: '<div class="drawer-stub" />' },
	stats: { template: '<div class="stats-stub" />' },
	teleport: true
}

describe('Configuration Filters', () => {
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
		it('should render AppInputAutocomplete for Type, Host, Endpoint, and Listener', async () => {
			const wrapper = mount(ItemIndex, { global: { stubs: globalStubs } })
			await flushPromises()

			const autocompletes = wrapper.findAllComponents({ name: 'AppInputAutocomplete' })
			expect(autocompletes.length).toBe(4)

			expect(autocompletes[0].props('label')).toBe('Type')
			expect(autocompletes[0].props('multiple')).toBe(true)
			expect(autocompletes[0].props('remote')).toBe(false)
			expect(autocompletes[0].props('clearable')).toBe(true)

			expect(autocompletes[1].props('label')).toBe('Host')
			expect(autocompletes[1].props('subLabelKey')).toBe('cluster_name')
			expect(autocompletes[1].props('multiple')).toBe(true)
			expect(autocompletes[1].props('remote')).toBe(true)
			expect(autocompletes[1].props('clearable')).toBe(true)

			expect(autocompletes[2].props('label')).toBe('Endpoint')
			expect(autocompletes[2].props('subLabelKey')).toBe('cluster_name')
			expect(autocompletes[2].props('multiple')).toBe(true)
			expect(autocompletes[2].props('remote')).toBe(true)
			expect(autocompletes[2].props('clearable')).toBe(true)

			expect(autocompletes[3].props('label')).toBe('Listener')
			expect(autocompletes[3].props('subLabelKey')).toBe('cluster_name')
			expect(autocompletes[3].props('multiple')).toBe(true)
			expect(autocompletes[3].props('remote')).toBe(true)
			expect(autocompletes[3].props('clearable')).toBe(true)
		})

		it('should update itemsStore.filter and trigger remote host, endpoint, and listener search', async () => {
			const store = useItemsStore()
			const fetchHostsSpy = vi.spyOn(store, 'fetchHosts')
			const fetchEndpointsSpy = vi.spyOn(store, 'fetchEndpoints')
			const fetchListenersSpy = vi.spyOn(store, 'fetchListeners')
			const wrapper = mount(ItemIndex, { global: { stubs: globalStubs } })
			await flushPromises()

			const filterDropdown = wrapper.findComponent({ name: 'AppFilterDropdown' })
			expect(filterDropdown.props('active')).toBe(false)

			const autocompletes = wrapper.findAllComponents({ name: 'AppInputAutocomplete' })

			await autocompletes[0].vm.$emit('update:modelValue', ['snmp', 'http'])
			await autocompletes[2].vm.$emit('update:modelValue', ['ep-123', 'ep-456'])
			await autocompletes[3].vm.$emit('update:modelValue', ['ls-111', 'ls-222'])
			await flushPromises()

			expect(filterDropdown.props('active')).toBe(true)
			expect(store.filter.types).toEqual(['snmp', 'http'])
			expect(store.filter.host_ids).toBeUndefined()
			expect(store.filter.endpoint_ids).toEqual(['ep-123', 'ep-456'])
			expect(store.filter.listener_ids).toEqual(['ls-111', 'ls-222'])

			await autocompletes[1].vm.$emit('search', 'alpha')
			await autocompletes[2].vm.$emit('search', 'router')
			await autocompletes[3].vm.$emit('search', 'trap')
			await flushPromises()
			expect(fetchHostsSpy).toHaveBeenCalledWith(undefined, 'alpha')
			expect(fetchEndpointsSpy).toHaveBeenCalledWith(undefined, 'router', undefined)
			expect(fetchListenersSpy).toHaveBeenCalledWith(undefined, 'trap')

			await filterDropdown.vm.$emit('reset')
			await flushPromises()
			expect(filterDropdown.props('active')).toBe(false)
			expect(store.filter.types).toBeUndefined()
			expect(store.filter.host_ids).toBeUndefined()
			expect(store.filter.endpoint_ids).toBeUndefined()
			expect(store.filter.listener_ids).toBeUndefined()
		})

		it('should chain host filter with endpoint options, remote search, and prune mismatched endpoints', async () => {
			const store = useItemsStore()
			const fetchEndpointsSpy = vi.spyOn(store, 'fetchEndpoints')
			const fetchItemsSpy = vi.spyOn(store, 'fetchItems')
			const wrapper = mount(ItemIndex, { global: { stubs: globalStubs } })
			await flushPromises()

			store.selection.endpoints = [
				{
					endpoint_id: 'ep-a1',
					host_id: 'host-1',
					host_name: 'Host Alpha',
					name: 'EP Alpha 1',
					adm_status: 1,
					last_updated: '',
					label: 'EP Alpha 1',
					value: 'ep-a1'
				},
				{
					endpoint_id: 'ep-b1',
					host_id: 'host-2',
					host_name: 'Host Beta',
					name: 'EP Beta 1',
					adm_status: 1,
					last_updated: '',
					label: 'EP Beta 1',
					value: 'ep-b1'
				}
			]
			await flushPromises()

			const autocompletes = wrapper.findAllComponents({ name: 'AppInputAutocomplete' })
			await autocompletes[2].vm.$emit('update:modelValue', ['ep-a1', 'ep-b1'])
			await flushPromises()

			fetchEndpointsSpy.mockClear()
			fetchItemsSpy.mockClear()

			// Selecting host-1 should prune ep-b1, keep ep-a1, refetch endpoints for host-1, and fetch items once
			await autocompletes[1].vm.$emit('update:modelValue', ['host-1'])
			await flushPromises()

			expect(store.filter.host_ids).toEqual(['host-1'])
			expect(store.filter.endpoint_ids).toEqual(['ep-a1'])
			expect(fetchEndpointsSpy).toHaveBeenCalledWith(undefined, undefined, ['host-1'])
			expect(fetchItemsSpy).toHaveBeenCalledTimes(1)

			// Searching endpoints while host-1 is active forwards host_ids
			fetchEndpointsSpy.mockClear()
			await autocompletes[2].vm.$emit('search', 'alpha')
			await flushPromises()
			expect(fetchEndpointsSpy).toHaveBeenCalledWith(undefined, 'alpha', ['host-1'])

			// Clearing host selection keeps currently selected endpoints and refetches unfiltered endpoints
			fetchEndpointsSpy.mockClear()
			await autocompletes[1].vm.$emit('update:modelValue', [])
			await flushPromises()
			expect(store.filter.host_ids).toBeUndefined()
			expect(store.filter.endpoint_ids).toEqual(['ep-a1'])
			expect(fetchEndpointsSpy).toHaveBeenCalledWith(undefined, undefined, undefined)
		})

		it('should reset itemsStore filter and query state on unmount without extra fetch', async () => {
			const store = useItemsStore()
			const fetchItemsSpy = vi.spyOn(store, 'fetchItems')
			const wrapper = mount(ItemIndex, { global: { stubs: globalStubs } })
			await flushPromises()

			const autocompletes = wrapper.findAllComponents({ name: 'AppInputAutocomplete' })
			await autocompletes[0].vm.$emit('update:modelValue', ['snmp'])
			await autocompletes[1].vm.$emit('update:modelValue', ['host-1'])
			await autocompletes[2].vm.$emit('update:modelValue', ['ep-123'])
			await autocompletes[3].vm.$emit('update:modelValue', ['ls-111'])
			await flushPromises()

			store.filter.name = 'cpu'
			store.filter.order_by = 'name'
			store.filter.order_dir = 'asc'
			store.filter.offset = 20
			store.pagination.currentPage = 3
			const initialLimit = store.filter.limit

			fetchItemsSpy.mockClear()
			wrapper.unmount()
			await flushPromises()

			expect(store.filter.types).toBeUndefined()
			expect(store.filter.host_ids).toBeUndefined()
			expect(store.filter.endpoint_ids).toBeUndefined()
			expect(store.filter.listener_ids).toBeUndefined()
			expect(store.filter.name).toBeUndefined()
			expect(store.filter.order_by).toBeUndefined()
			expect(store.filter.order_dir).toBeUndefined()
			expect(store.filter.offset).toBe(0)
			expect(store.pagination.currentPage).toBe(1)
			expect(store.filter.exclusivity).toEqual(['host', 'template_on_host'])
			expect(store.filter.collect_item_stats).toBe(true)
			expect(store.filter.limit).toBe(initialLimit)
			expect(fetchItemsSpy).not.toHaveBeenCalled()
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

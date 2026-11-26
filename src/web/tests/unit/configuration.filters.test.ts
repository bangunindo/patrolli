// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, it, expect, beforeEach, vi } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import '@/prototypes/string.prototype'
import appHelper from '@/app/helpers/app.helper'
import endpointsApi from '@/app/api/endpoints.api'
import hostsApi from '@/app/api/hosts.api'
import listenersApi from '@/app/api/listeners.api'
import nodesApi from '@/app/api/nodes.api'
import sinksApi from '@/app/api/sinks.api'
import { useEndpointsStore } from '@/stores/endpoints.store'
import { useHostsStore } from '@/stores/hosts.store'
import { useListenerStore } from '@/stores/listeners.store'
import { useItemsStore } from '@/stores/items.store'
import { useSinksStore } from '@/stores/sinks.store'
import { usePipelineStore } from '@/stores/pipelines.store'
import { useTemplateItemsStore } from '@/stores/template-items.store'
import { useTemplatesStore } from '@/stores/templates.store'
import { useUsersStore } from '@/stores/users.store'

vi.stubGlobal('definePageMeta', vi.fn())

const mockRoute = {
	query: {} as Record<string, any>,
	params: {} as Record<string, any>,
	path: '/configuration/item'
}

vi.mock('vue-router', () => ({
	useRoute: () => mockRoute,
	useRouter: () => ({
		push: vi.fn(),
		replace: vi.fn()
	})
}))

import EndpointForm from '@/components/configuration/endpoint/Form.vue'
import EndpointTable from '@/components/configuration/endpoint/Table.vue'
import HostTable from '@/components/configuration/host/Table.vue'
import ListenerTable from '@/components/configuration/listener/Table.vue'
import SinkTable from '@/components/configuration/sinks/Table.vue'
import TemplateItemsTable from '@/components/configuration/template-items/Table.vue'
import EndpointIndex from '@/pages/configuration/endpoint/index.vue'
import HostIndex from '@/pages/configuration/host/index.vue'
import ListenerIndex from '@/pages/configuration/listener/index.vue'
import ItemIndex from '@/pages/configuration/item/index.vue'
import SinkIndex from '@/pages/configuration/sink/index.vue'
import PipelineIndex from '@/pages/pipeline/index.vue'
import TemplateIndex from '@/pages/configuration/template/index.vue'
import TemplateItemsIndex from '@/pages/configuration/template/[template_id]/items.vue'
import UserManagementIndex from '@/pages/user-management/index.vue'

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
				nodes: [],
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
	UiBadge: { template: '<span><slot /></span>' },
	UiIcon: { template: '<span class="icon-stub" />' },
	UiInput: { template: '<input />' },
	UiDropdownMenu: { template: '<div class="dropdown"><slot /></div>' },
	UiDropdownMenuTrigger: { template: '<div><slot /></div>' },
	UiDropdownMenuContent: { template: '<div class="dropdown-content"><slot /></div>' },
	UiDropdownMenuItem: { template: '<div><slot /></div>' },
	UiDropdownMenuSeparator: { template: '<hr />' },
	AppTable: { template: '<div class="app-table-stub" />' },
	AppInputCluster: { template: '<div class="cluster-input-stub" />' },
	AppInputHost: { template: '<div class="host-input-stub" />' },
	DynamicZodForm: { template: '<div><slot name="field:host_id" /></div>' },
	ModalDetail: { template: '<div class="modal-detail-stub" />' },
	ModalEndpointForm: { template: '<div class="modal-endpoint-form-stub" />' },
	ModalHostForm: { template: '<div class="modal-host-form-stub" />' },
	ModalListenerForm: { template: '<div class="modal-listener-form-stub" />' },
	ModalSinkForm: { template: '<div class="modal-sink-form-stub" />' },
	ModalForm: { template: '<div class="modal-form-stub" />' },
	ModalConfirmationDelete: { template: '<div class="modal-delete-stub" />' },
	ToggleTemplate: { template: '<div class="toggle-template-stub" />' },
	'endpoint-table': { template: '<div class="endpoint-table-stub" />' },
	'host-table': { template: '<div class="host-table-stub" />' },
	'listener-table': { template: '<div class="listener-table-stub" />' },
	'items-table': { template: '<div class="items-table-stub" />' },
	'sink-table': { template: '<div class="sink-table-stub" />' },
	'template-table': { template: '<div class="template-table-stub" />' },
	'template-items-table': { template: '<div class="template-items-table-stub" />' },
	'pipeline-table': { template: '<div class="pipeline-table-stub" />' },
	'modal-form': { template: '<div class="modal-form-stub" />' },
	UserStatsCards: { template: '<div class="user-stats-stub" />' },
	UserToolbar: { template: '<div class="user-toolbar-stub" />' },
	UserTable: { template: '<div class="user-table-stub" />' },
	ModalUserDetail: { template: '<div class="modal-user-detail-stub" />' },
	ModalUserForm: { template: '<div class="modal-user-form-stub" />' },
	ModalDeleteUser: { template: '<div class="modal-delete-user-stub" />' },
	'config-drawer': { template: '<div class="drawer-stub" />' },
	stats: { template: '<div class="stats-stub" />' },
	teleport: true
}

describe('Configuration Filters', () => {
	beforeEach(() => {
		setActivePinia(createPinia())
		mockRoute.query = {}
		mockRoute.params = {}
		mockRoute.path = '/configuration/item'
		vi.restoreAllMocks()
		vi.clearAllMocks()
	})

	describe('Hosts Filter', () => {
		it('should render inline AppInputAutocomplete for Cluster Node', async () => {
			const wrapper = mount(HostTable, { global: { stubs: globalStubs } })
			await flushPromises()

			const autocompletes = wrapper.findAllComponents({ name: 'AppInputAutocomplete' })
			expect(autocompletes.length).toBe(1)

			expect(autocompletes[0].props('placeholder')).toBe('Select Node')
			expect(autocompletes[0].props('subLabelKey')).toBe('cluster_name')
			expect(autocompletes[0].props('multiple')).toBe(false)
			expect(autocompletes[0].props('remote')).toBe(false)
			expect(autocompletes[0].props('clearable')).toBe(true)
		})

		it('should update hostsStore.filter.monitored_by and normalize clear to undefined', async () => {
			const store = useHostsStore()
			const fetchHostsSpy = vi.spyOn(store, 'fetchHosts')
			const wrapper = mount(HostTable, { global: { stubs: globalStubs } })
			await flushPromises()

			fetchHostsSpy.mockClear()
			const autocomplete = wrapper.findComponent({ name: 'AppInputAutocomplete' })

			await autocomplete.vm.$emit('update:modelValue', 'node-1')
			await flushPromises()
			expect(store.filter.monitored_by).toBe('node-1')
			expect(store.pagination.currentPage).toBe(1)
			expect(store.filter.offset).toBe(0)
			expect(fetchHostsSpy).toHaveBeenCalledTimes(1)

			await autocomplete.vm.$emit('update:modelValue', '')
			await flushPromises()
			expect(store.filter.monitored_by).toBeUndefined()
			expect(fetchHostsSpy).toHaveBeenCalledTimes(2)
		})

		it('should reset hostsStore filter and query state on unmount without extra fetch', async () => {
			const store = useHostsStore()
			const fetchHostsSpy = vi.spyOn(store, 'fetchHosts')
			const wrapper = mount(HostIndex, { global: { stubs: globalStubs } })
			await flushPromises()

			store.filter.monitored_by = 'node-1'
			store.filter.name = 'web-01'
			store.filter.order_by = 'name'
			store.filter.order_dir = 'asc'
			store.filter.offset = 20
			store.pagination.currentPage = 3
			store.currentHost = { host_id: 'host-1', name: 'Web 01' } as any
			const initialLimit = store.filter.limit

			fetchHostsSpy.mockClear()
			wrapper.unmount()
			await flushPromises()

			expect(store.filter.monitored_by).toBeUndefined()
			expect(store.filter.name).toBeUndefined()
			expect(store.filter.order_by).toBeUndefined()
			expect(store.filter.order_dir).toBeUndefined()
			expect(store.filter.offset).toBe(0)
			expect(store.pagination.currentPage).toBe(1)
			expect(store.filter.limit).toBe(initialLimit)
			expect(store.currentHost).toBeNull()
			expect(fetchHostsSpy).not.toHaveBeenCalled()
		})
	})

	describe('Endpoints Filter', () => {
		it('should render AppInputAutocomplete for Type and Host', async () => {
			const wrapper = mount(EndpointIndex, { global: { stubs: globalStubs } })
			await flushPromises()

			const autocompletes = wrapper.findAllComponents({ name: 'AppInputAutocomplete' })
			expect(autocompletes.length).toBe(2)

			expect(autocompletes[0].props('label')).toBe('Type')
			expect(autocompletes[0].props('multiple')).toBe(true)
			expect(autocompletes[0].props('remote')).toBe(false)
			expect(autocompletes[0].props('clearable')).toBe(true)

			expect(autocompletes[1].props('label')).toBe('Host')
			expect(autocompletes[1].props('subLabelKey')).toBe('cluster_name')
			expect(autocompletes[1].props('multiple')).toBe(true)
			expect(autocompletes[1].props('remote')).toBe(true)
			expect(autocompletes[1].props('clearable')).toBe(true)
		})

		it('should update endpointsStore.filter and trigger remote host search', async () => {
			const store = useEndpointsStore()
			const fetchHostsSpy = vi.spyOn(store, 'fetchHosts')
			const wrapper = mount(EndpointIndex, { global: { stubs: globalStubs } })
			await flushPromises()

			const filterDropdown = wrapper.findComponent({ name: 'AppFilterDropdown' })
			expect(filterDropdown.props('active')).toBe(false)

			const autocompletes = wrapper.findAllComponents({ name: 'AppInputAutocomplete' })

			await autocompletes[0].vm.$emit('update:modelValue', ['snmp_v2c', 'http'])
			await autocompletes[1].vm.$emit('update:modelValue', ['host-1', 'host-2'])
			await flushPromises()

			expect(filterDropdown.props('active')).toBe(true)
			expect(store.filter.types).toEqual(['snmp_v2c', 'http'])
			expect(store.filter.host_ids).toEqual(['host-1', 'host-2'])

			await autocompletes[1].vm.$emit('search', 'alpha')
			await flushPromises()
			expect(fetchHostsSpy).toHaveBeenCalledWith(undefined, 'alpha')

			await filterDropdown.vm.$emit('reset')
			await flushPromises()
			expect(filterDropdown.props('active')).toBe(false)
			expect(store.filter.types).toBeUndefined()
			expect(store.filter.host_ids).toBeUndefined()
		})

		it('should reset endpointsStore filter and query state on unmount without extra fetch', async () => {
			const store = useEndpointsStore()
			const fetchEndpointsSpy = vi.spyOn(store, 'fetchEndpoints')
			const wrapper = mount(EndpointIndex, { global: { stubs: globalStubs } })
			await flushPromises()

			const autocompletes = wrapper.findAllComponents({ name: 'AppInputAutocomplete' })
			await autocompletes[0].vm.$emit('update:modelValue', ['snmp_v2c'])
			await autocompletes[1].vm.$emit('update:modelValue', ['host-1'])
			await flushPromises()

			store.filter.name = 'router'
			store.filter.order_by = 'name'
			store.filter.order_dir = 'asc'
			store.filter.offset = 20
			store.pagination.currentPage = 3
			const initialLimit = store.filter.limit

			fetchEndpointsSpy.mockClear()
			wrapper.unmount()
			await flushPromises()

			expect(store.filter.types).toBeUndefined()
			expect(store.filter.host_ids).toBeUndefined()
			expect(store.filter.name).toBeUndefined()
			expect(store.filter.order_by).toBeUndefined()
			expect(store.filter.order_dir).toBeUndefined()
			expect(store.filter.offset).toBe(0)
			expect(store.pagination.currentPage).toBe(1)
			expect(store.filter.limit).toBe(initialLimit)
			expect(fetchEndpointsSpy).not.toHaveBeenCalled()
		})
	})

	describe('Listeners Filter', () => {
		it('should render AppInputAutocomplete for Type', async () => {
			const wrapper = mount(ListenerIndex, { global: { stubs: globalStubs } })
			await flushPromises()

			const autocompletes = wrapper.findAllComponents({ name: 'AppInputAutocomplete' })
			expect(autocompletes.length).toBe(1)
			expect(autocompletes[0].props('label')).toBe('Type')
			expect(autocompletes[0].props('placeholder')).toBe('All Types')
			expect(autocompletes[0].props('multiple')).toBe(true)
			expect(autocompletes[0].props('remote')).toBe(false)
			expect(autocompletes[0].props('clearable')).toBe(true)
		})

		it('should update listenersStore.filter and reset via AppFilterDropdown', async () => {
			const store = useListenerStore()
			const wrapper = mount(ListenerIndex, { global: { stubs: globalStubs } })
			await flushPromises()

			const filterDropdown = wrapper.findComponent({ name: 'AppFilterDropdown' })
			expect(filterDropdown.props('active')).toBe(false)

			const autocomplete = wrapper.findComponent({ name: 'AppInputAutocomplete' })
			await autocomplete.vm.$emit('update:modelValue', ['snmp_trap', 'syslog'])
			await flushPromises()

			expect(filterDropdown.props('active')).toBe(true)
			expect(store.filter.types).toEqual(['snmp_trap', 'syslog'])

			await filterDropdown.vm.$emit('reset')
			await flushPromises()

			expect(filterDropdown.props('active')).toBe(false)
			expect(store.filter.types).toBeUndefined()
		})

		it('should reset listenersStore filter and query state on unmount without extra fetch', async () => {
			const store = useListenerStore()
			const fetchListenersSpy = vi.spyOn(store, 'fetchListeners')
			const wrapper = mount(ListenerIndex, { global: { stubs: globalStubs } })
			await flushPromises()

			const autocomplete = wrapper.findComponent({ name: 'AppInputAutocomplete' })
			await autocomplete.vm.$emit('update:modelValue', ['snmp_trap'])
			await flushPromises()

			store.filter.name = 'trap'
			store.filter.order_by = 'name'
			store.filter.order_dir = 'asc'
			store.filter.offset = 20
			store.pagination.currentPage = 3
			const initialLimit = store.filter.limit

			fetchListenersSpy.mockClear()
			wrapper.unmount()
			await flushPromises()

			expect(store.filter.types).toBeUndefined()
			expect(store.filter.name).toBeUndefined()
			expect(store.filter.order_by).toBeUndefined()
			expect(store.filter.order_dir).toBeUndefined()
			expect(store.filter.offset).toBe(0)
			expect(store.pagination.currentPage).toBe(1)
			expect(store.filter.limit).toBe(initialLimit)
			expect(fetchListenersSpy).not.toHaveBeenCalled()
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
		it('should render AppInputAutocomplete for Type', async () => {
			const wrapper = mount(SinkIndex, { global: { stubs: globalStubs } })
			await flushPromises()

			const autocompletes = wrapper.findAllComponents({ name: 'AppInputAutocomplete' })
			expect(autocompletes.length).toBe(1)
			expect(autocompletes[0].props('label')).toBe('Type')
			expect(autocompletes[0].props('placeholder')).toBe('All Types')
			expect(autocompletes[0].props('multiple')).toBe(true)
			expect(autocompletes[0].props('remote')).toBe(false)
			expect(autocompletes[0].props('clearable')).toBe(true)
		})

		it('should update sinksStore.filter and reset via AppFilterDropdown', async () => {
			const store = useSinksStore()
			const wrapper = mount(SinkIndex, { global: { stubs: globalStubs } })
			await flushPromises()

			const filterDropdown = wrapper.findComponent({ name: 'AppFilterDropdown' })
			expect(filterDropdown.props('active')).toBe(false)

			const autocomplete = wrapper.findComponent({ name: 'AppInputAutocomplete' })
			await autocomplete.vm.$emit('update:modelValue', ['kafka', 'clickhouse'])
			await flushPromises()

			expect(filterDropdown.props('active')).toBe(true)
			expect(store.filter.types).toEqual(['kafka', 'clickhouse'])

			await filterDropdown.vm.$emit('reset')
			await flushPromises()

			expect(filterDropdown.props('active')).toBe(false)
			expect(store.filter.types).toBeUndefined()
		})

		it('should reset sinksStore filter and query state on unmount without extra fetch', async () => {
			const store = useSinksStore()
			const fetchSinksSpy = vi.spyOn(store, 'fetchSinks')
			const wrapper = mount(SinkIndex, { global: { stubs: globalStubs } })
			await flushPromises()

			const autocomplete = wrapper.findComponent({ name: 'AppInputAutocomplete' })
			await autocomplete.vm.$emit('update:modelValue', ['kafka'])
			await flushPromises()

			store.filter.name = 'events'
			store.filter.order_by = 'name'
			store.filter.order_dir = 'asc'
			store.filter.offset = 20
			store.pagination.currentPage = 3
			const initialLimit = store.filter.limit

			fetchSinksSpy.mockClear()
			wrapper.unmount()
			await flushPromises()

			expect(store.filter.types).toBeUndefined()
			expect(store.filter.name).toBeUndefined()
			expect(store.filter.order_by).toBeUndefined()
			expect(store.filter.order_dir).toBeUndefined()
			expect(store.filter.offset).toBe(0)
			expect(store.pagination.currentPage).toBe(1)
			expect(store.filter.limit).toBe(initialLimit)
			expect(fetchSinksSpy).not.toHaveBeenCalled()
		})
	})

	describe('Templates Cleanup', () => {
		it('should reset templatesStore filter and currentTemplate on unmount without extra fetch', async () => {
			const store = useTemplatesStore()
			const fetchTemplatesSpy = vi.spyOn(store, 'fetchTemplates')
			const wrapper = mount(TemplateIndex, { global: { stubs: globalStubs } })
			await flushPromises()

			store.filter.name = 'linux'
			store.filter.order_by = 'name'
			store.filter.order_dir = 'asc'
			store.filter.offset = 20
			store.pagination.currentPage = 3
			store.currentTemplate = { template_id: 'tmpl-1', name: 'Linux' } as any
			const initialLimit = store.filter.limit

			fetchTemplatesSpy.mockClear()
			wrapper.unmount()
			await flushPromises()

			expect(store.filter.name).toBeUndefined()
			expect(store.filter.order_by).toBeUndefined()
			expect(store.filter.order_dir).toBeUndefined()
			expect(store.filter.offset).toBe(0)
			expect(store.pagination.currentPage).toBe(1)
			expect(store.filter.limit).toBe(initialLimit)
			expect(store.currentTemplate).toBeNull()
			expect(fetchTemplatesSpy).not.toHaveBeenCalled()
		})
	})

	describe('Template Items Filter', () => {
		it('should render AppInputAutocomplete for Type only', async () => {
			const wrapper = mount(TemplateItemsIndex, { global: { stubs: globalStubs } })
			await flushPromises()

			const autocompletes = wrapper.findAllComponents({ name: 'AppInputAutocomplete' })
			expect(autocompletes.length).toBe(1)

			expect(autocompletes[0].props('label')).toBe('Type')
			expect(autocompletes[0].props('multiple')).toBe(true)
			expect(autocompletes[0].props('remote')).toBe(false)
			expect(autocompletes[0].props('clearable')).toBe(true)
		})

		it('should update templateItemsStore.filter.types and reset via AppFilterDropdown', async () => {
			const store = useTemplateItemsStore()
			const wrapper = mount(TemplateItemsIndex, { global: { stubs: globalStubs } })
			await flushPromises()

			const filterDropdown = wrapper.findComponent({ name: 'AppFilterDropdown' })
			expect(filterDropdown.props('active')).toBe(false)

			const autocomplete = wrapper.findComponent({ name: 'AppInputAutocomplete' })
			await autocomplete.vm.$emit('update:modelValue', ['snmp', 'http'])
			await flushPromises()

			expect(filterDropdown.props('active')).toBe(true)
			expect(store.filter.types).toEqual(['snmp', 'http'])

			await filterDropdown.vm.$emit('reset')
			await flushPromises()
			expect(filterDropdown.props('active')).toBe(false)
			expect(store.filter.types).toBeUndefined()
		})

		it('should reset templateItemsStore filter and query state on unmount without extra fetch', async () => {
			const store = useTemplateItemsStore()
			const fetchItemsSpy = vi.spyOn(store, 'fetchItems')
			const wrapper = mount(TemplateItemsIndex, { global: { stubs: globalStubs } })
			await flushPromises()

			const autocomplete = wrapper.findComponent({ name: 'AppInputAutocomplete' })
			await autocomplete.vm.$emit('update:modelValue', ['snmp'])
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
			expect(store.filter.name).toBeUndefined()
			expect(store.filter.order_by).toBeUndefined()
			expect(store.filter.order_dir).toBeUndefined()
			expect(store.filter.offset).toBe(0)
			expect(store.pagination.currentPage).toBe(1)
			expect(store.filter.exclusivity).toEqual(['template'])
			expect(store.filter.limit).toBe(initialLimit)
			expect(fetchItemsSpy).not.toHaveBeenCalled()
		})
	})

	describe('Pipeline Cleanup', () => {
		it('should reset pipelineStore filter and currentPipeline on unmount without extra fetch', async () => {
			const store = usePipelineStore()
			const fetchPipelinesSpy = vi.spyOn(store, 'fetchPipelines')
			const wrapper = mount(PipelineIndex, { global: { stubs: globalStubs } })
			await flushPromises()

			store.filter.name = 'telemetry'
			store.filter.order_by = 'name'
			store.filter.order_dir = 'asc'
			store.filter.offset = 20
			store.pagination.currentPage = 3
			store.currentPipeline = { id: 'pipe-1', name: 'Telemetry' } as any
			const initialLimit = store.filter.limit

			fetchPipelinesSpy.mockClear()
			wrapper.unmount()
			await flushPromises()

			expect(store.filter.name).toBeUndefined()
			expect(store.filter.order_by).toBeUndefined()
			expect(store.filter.order_dir).toBeUndefined()
			expect(store.filter.offset).toBe(0)
			expect(store.pagination.currentPage).toBe(1)
			expect(store.filter.limit).toBe(initialLimit)
			expect(store.currentPipeline).toBeNull()
			expect(fetchPipelinesSpy).not.toHaveBeenCalled()
		})
	})

	describe('User Management Cleanup', () => {
		it('should reset usersStore filter and currentUser on unmount without extra fetch', async () => {
			const store = useUsersStore()
			const fetchUsersSpy = vi.spyOn(store, 'fetchUsers').mockResolvedValue(undefined)
			const wrapper = mount(UserManagementIndex, { global: { stubs: globalStubs } })
			await flushPromises()

			store.filter.username = 'admin'
			store.filter.order_by = 'username'
			store.filter.order_dir = 'asc'
			store.filter.offset = 20
			store.pagination.currentPage = 3
			store.currentUser = { user_id: 'user-1', username: 'admin' } as any
			const initialLimit = store.filter.limit

			fetchUsersSpy.mockClear()
			wrapper.unmount()
			await flushPromises()

			expect(store.filter.username).toBeUndefined()
			expect(store.filter.order_by).toBeUndefined()
			expect(store.filter.order_dir).toBeUndefined()
			expect(store.filter.offset).toBe(0)
			expect(store.pagination.currentPage).toBe(1)
			expect(store.filter.limit).toBe(initialLimit)
			expect(store.currentUser).toBeNull()
			expect(fetchUsersSpy).not.toHaveBeenCalled()
		})
	})

	describe('Review Fix Regressions', () => {
		it('should not call endpointsStore.fetchHosts when mounting EndpointForm', async () => {
			const store = useEndpointsStore()
			const fetchHostsSpy = vi.spyOn(store, 'fetchHosts')

			mount(EndpointForm, { global: { stubs: globalStubs } })
			await flushPromises()

			expect(fetchHostsSpy).not.toHaveBeenCalled()
		})

		it('should clear selection.hosts and ignore stale in-flight fetchHosts after resetFilter', async () => {
			const store = useEndpointsStore()
			let resolveHosts!: (value: any) => void
			vi.spyOn(hostsApi, 'getHosts').mockImplementationOnce(
				() =>
					new Promise((resolve) => {
						resolveHosts = resolve
					})
			)

			const pendingFetch = store.fetchHosts(undefined, 'alpha')
			expect(store.loading.hosts).toBe(true)

			store.resetFilter()
			expect(store.loading.hosts).toBe(false)
			expect(store.selection.hosts).toEqual([])

			resolveHosts({
				status_code: 200,
				success: true,
				data: {
					hosts: [{ host_id: 'host-stale', name: 'Stale Host' }]
				}
			})
			await pendingFetch

			expect(store.selection.hosts).toEqual([])
			expect(store.loading.hosts).toBe(false)
		})

		it('should pass undefined clusterURLs in TemplateItemsTable when cluster query is missing and [url] when resolved', async () => {
			const store = useTemplateItemsStore()
			const fetchItemsSpy = vi.spyOn(store, 'fetchItems').mockResolvedValue(undefined)
			vi.spyOn(appHelper, 'getClusterValue').mockReturnValueOnce('')

			const wrapperEmpty = mount(TemplateItemsTable, { global: { stubs: globalStubs } })
			await flushPromises()
			expect(fetchItemsSpy).toHaveBeenCalledWith(undefined)
			wrapperEmpty.unmount()

			fetchItemsSpy.mockClear()
			mockRoute.query = { cluster: 'Cluster A' }
			vi.spyOn(appHelper, 'getClusterValue').mockReturnValueOnce('http://cluster-a.test')

			const wrapperWithCluster = mount(TemplateItemsTable, {
				global: { stubs: globalStubs }
			})
			await flushPromises()
			expect(fetchItemsSpy).toHaveBeenCalledWith(['http://cluster-a.test'])
			wrapperWithCluster.unmount()
		})

		it('should pass resolved clusterURLs from route.query.cluster in EndpointTable, ListenerTable, and SinkTable', async () => {
			mockRoute.query = { cluster: 'Cluster A' }
			vi.spyOn(appHelper, 'getClusterValue').mockReturnValue('http://cluster-a.test')

			const endpointsStore = useEndpointsStore()
			const listenersStore = useListenerStore()
			const sinksStore = useSinksStore()
			const fetchEndpointsSpy = vi
				.spyOn(endpointsStore, 'fetchEndpoints')
				.mockResolvedValue(undefined)
			const fetchListenersSpy = vi
				.spyOn(listenersStore, 'fetchListeners')
				.mockResolvedValue(undefined)
			const fetchSinksSpy = vi.spyOn(sinksStore, 'fetchSinks').mockResolvedValue(undefined)

			const epWrapper = mount(EndpointTable, { global: { stubs: globalStubs } })
			const lsWrapper = mount(ListenerTable, { global: { stubs: globalStubs } })
			const sinkWrapper = mount(SinkTable, { global: { stubs: globalStubs } })
			await flushPromises()

			expect(fetchEndpointsSpy).toHaveBeenCalledWith(['http://cluster-a.test'])
			expect(fetchListenersSpy).toHaveBeenCalledWith(['http://cluster-a.test'])
			expect(fetchSinksSpy).toHaveBeenCalledWith(['http://cluster-a.test'])

			epWrapper.unmount()
			lsWrapper.unmount()
			sinkWrapper.unmount()
		})

		it('should resolve cluster URLs via appHelper.getClusterUrls', () => {
			vi.spyOn(appHelper, 'getClusterValue').mockReturnValueOnce('')
			expect(appHelper.getClusterUrls(undefined)).toBeUndefined()

			vi.spyOn(appHelper, 'getClusterValue').mockReturnValueOnce('http://cluster-a.test')
			expect(appHelper.getClusterUrls('Cluster A')).toEqual(['http://cluster-a.test'])
		})

		it('should preserve activeClusterURLs on create/update refresh and clear on resetFilter for endpoints, listeners, and sinks stores', async () => {
			const clusterUrls = ['http://cluster-a.test']
			const endpointsStore = useEndpointsStore()
			const listenersStore = useListenerStore()
			const sinksStore = useSinksStore()

			const getEndpointsSpy = vi.spyOn(endpointsApi, 'getEndpoints').mockResolvedValue({
				status_code: 200,
				success: true,
				data: { endpoints: [], total: 0 }
			} as any)
			vi.spyOn(endpointsApi, 'createEndpoint').mockResolvedValue({ success: true } as any)
			vi.spyOn(endpointsApi, 'updateEndpoint').mockResolvedValue({ success: true } as any)

			const getListenersSpy = vi.spyOn(listenersApi, 'getListeners').mockResolvedValue({
				status_code: 200,
				success: true,
				data: { listeners: [], total: 0 }
			} as any)
			vi.spyOn(listenersApi, 'createListener').mockResolvedValue({ success: true } as any)
			vi.spyOn(listenersApi, 'updateListener').mockResolvedValue({ success: true } as any)

			const getSinksSpy = vi.spyOn(sinksApi, 'getSinks').mockResolvedValue({
				status_code: 200,
				success: true,
				data: { sinks: [], total: 0 }
			} as any)
			vi.spyOn(sinksApi, 'createSink').mockResolvedValue({ success: true } as any)
			vi.spyOn(sinksApi, 'updateSink').mockResolvedValue({ success: true } as any)

			await endpointsStore.fetchEndpoints(clusterUrls)
			await listenersStore.fetchListeners(clusterUrls)
			await sinksStore.fetchSinks(clusterUrls)
			getEndpointsSpy.mockClear()
			getListenersSpy.mockClear()
			getSinksSpy.mockClear()

			await endpointsStore.createEndpoint({} as any, ['http://cluster-b.test'])
			await endpointsStore.updateEndpoint('ep-1', {} as any, ['http://cluster-b.test'])
			expect(getEndpointsSpy).toHaveBeenNthCalledWith(1, endpointsStore.filter, clusterUrls)
			expect(getEndpointsSpy).toHaveBeenNthCalledWith(2, endpointsStore.filter, clusterUrls)

			await listenersStore.createListener({} as any, ['http://cluster-b.test'])
			await listenersStore.updateListener('ls-1', {} as any, ['http://cluster-b.test'])
			expect(getListenersSpy).toHaveBeenNthCalledWith(1, listenersStore.filter, clusterUrls)
			expect(getListenersSpy).toHaveBeenNthCalledWith(2, listenersStore.filter, clusterUrls)

			await sinksStore.createSink({} as any, ['http://cluster-b.test'])
			await sinksStore.updateSink('sk-1', {} as any, ['http://cluster-b.test'])
			expect(getSinksSpy).toHaveBeenNthCalledWith(1, sinksStore.filter, clusterUrls)
			expect(getSinksSpy).toHaveBeenNthCalledWith(2, sinksStore.filter, clusterUrls)

			endpointsStore.resetFilter()
			listenersStore.resetFilter()
			sinksStore.resetFilter()
			getEndpointsSpy.mockClear()
			getListenersSpy.mockClear()
			getSinksSpy.mockClear()

			await endpointsStore.createEndpoint({} as any, ['http://cluster-b.test'])
			await listenersStore.createListener({} as any, ['http://cluster-b.test'])
			await sinksStore.createSink({} as any, ['http://cluster-b.test'])

			expect(getEndpointsSpy).toHaveBeenCalledWith(endpointsStore.filter, undefined)
			expect(getListenersSpy).toHaveBeenCalledWith(listenersStore.filter, undefined)
			expect(getSinksSpy).toHaveBeenCalledWith(sinksStore.filter, undefined)
		})

		it('should clear selection.nodes and ignore stale in-flight fetchNodes after hostsStore.resetFilter', async () => {
			const store = useHostsStore()
			let resolveNodes!: (value: any) => void
			vi.spyOn(nodesApi, 'getNodes').mockImplementationOnce(
				() =>
					new Promise((resolve) => {
						resolveNodes = resolve
					})
			)

			const pendingFetch = store.fetchNodes()
			expect(store.loading.nodes).toBe(true)

			store.resetFilter()
			expect(store.loading.nodes).toBe(false)
			expect(store.selection.nodes).toEqual([])

			resolveNodes({
				status_code: 200,
				success: true,
				data: {
					nodes: [{ node_id: 'node-stale', name: 'Stale Node' }]
				}
			})
			await pendingFetch

			expect(store.selection.nodes).toEqual([])
			expect(store.loading.nodes).toBe(false)
		})
	})
})

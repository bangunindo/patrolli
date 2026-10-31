// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { reactive } from 'vue'
import { mount, flushPromises } from '@vue/test-utils'
import { setActivePinia, createPinia } from 'pinia'
import { usePipelineStore } from '@/stores/pipeline.store'
import { useAppStore } from '@/stores/app.store'
import pipelineManagementApi from '@/app/api/pipeline.management.api'
import hostsApi from '@/app/api/hosts.api'
import sinksApi from '@/app/api/sinks.api'
import appHelper from '@/app/helpers/app.helper'

const mockRoute = reactive<{ path: string; query: Record<string, any> }>({
	path: '/pipeline',
	query: {}
})

vi.mock('vue-router', async (importOriginal) => {
	const actual = await importOriginal<any>()
	return {
		...actual,
		useRoute: () => mockRoute,
		useRouter: () => ({ push: vi.fn() })
	}
})

describe('Pipeline Store & Multicluster Integration', () => {
	beforeEach(() => {
		setActivePinia(createPinia())
		vi.clearAllMocks()

		vi.spyOn(appHelper, 'getBaseURLs').mockReturnValue([
			{
				alias: 'cluster-1',
				name: 'Cluster 1',
				label: 'Cluster 1',
				value: 'http://c1.test:8080'
			},
			{
				alias: 'cluster-2',
				name: 'Cluster 2',
				label: 'Cluster 2',
				value: 'http://c2.test:8080'
			}
		] as any)

		vi.spyOn(appHelper, 'getDefaultClusterValue').mockReturnValue('http://c1.test:8080')
		vi.spyOn(appHelper, 'getClusterName').mockImplementation((url?: string) => {
			if (url === 'http://c1.test:8080') return 'Cluster 1'
			if (url === 'http://c2.test:8080') return 'Cluster 2'
			return url || ''
		})
	})

	it('initializes default form with default cluster and empty selections', () => {
		const store = usePipelineStore()
		expect(store.form.cluster_url).toBe('http://c1.test:8080')
		expect(store.form.name).toBe('')
		expect(store.form.access).toBe('public')
		expect(store.form.visibility).toBe(1)
		expect(store.form.assignedHosts).toEqual([])
		expect(store.form.assignedSinks).toEqual([])
	})

	it('fetches pipelines across all available clusters', async () => {
		const getPipelinesSpy = vi
			.spyOn(pipelineManagementApi, 'getPipelines')
			.mockResolvedValueOnce({
				success: true,
				data: {
					pipeline_managements: [
						{
							pipeline_management_id: 'pipe-uuid-1',
							name: 'Pipeline One',
							adm_status: 1,
							visibility: 1,
							created_at: new Date().toISOString(),
							last_updated: new Date().toISOString(),
							cluster_url: 'http://c1.test:8080',
							cluster_name: 'Cluster 1'
						},
						{
							pipeline_management_id: 'pipe-uuid-2',
							name: 'Pipeline Two',
							adm_status: 0,
							visibility: 0,
							created_at: new Date().toISOString(),
							last_updated: new Date().toISOString(),
							cluster_url: 'http://c2.test:8080',
							cluster_name: 'Cluster 2'
						}
					] as any,
					total: 2
				}
			} as any)

		const store = usePipelineStore()
		await store.fetchPipelines()

		expect(getPipelinesSpy).toHaveBeenCalledWith(store.filter, undefined)
		expect(store.pipelines.length).toBe(2)
		expect(store.pagination.totalRecords).toBe(2)
		expect(store.pipelines[0].cluster_name).toBe('Cluster 1')
		expect(store.pipelines[0].status).toBe('running')
		expect(store.pipelines[0].access).toBe('public')
		expect(store.pipelines[1].cluster_name).toBe('Cluster 2')
		expect(store.pipelines[1].status).toBe('stopped')
		expect(store.pipelines[1].access).toBe('private')
	})

	it('fetches pipelines only for the active cluster when activeClusterURL is set', async () => {
		const appStore = useAppStore()
		appStore.activeClusterURL = 'http://c2.test:8080'

		const getPipelinesSpy = vi.spyOn(pipelineManagementApi, 'getPipelines').mockResolvedValue({
			success: true,
			data: {
				pipeline_managements: [
					{
						pipeline_management_id: 'pipe-uuid-2',
						name: 'Pipeline Two',
						adm_status: 1,
						visibility: 1,
						created_at: new Date().toISOString(),
						last_updated: new Date().toISOString(),
						cluster_url: 'http://c2.test:8080',
						cluster_name: 'Cluster 2'
					}
				] as any,
				total: 1
			}
		} as any)

		const store = usePipelineStore()
		await store.fetchPipelines()

		expect(getPipelinesSpy).toHaveBeenCalledTimes(1)
		expect(getPipelinesSpy).toHaveBeenCalledWith(store.filter, undefined)
		expect(store.pipelines.length).toBe(1)
		expect(store.pipelines[0].cluster_name).toBe('Cluster 2')
	})

	it('fetches hosts dynamically for a cluster using hostsApi', async () => {
		vi.spyOn(hostsApi, 'getHosts').mockResolvedValue({
			success: true,
			data: {
				hosts: [
					{ host_id: 'host-1', name: 'Host Alpha', ip: '192.168.1.1' },
					{ host_id: 'host-2', name: 'Host Beta', ip: '192.168.1.2' }
				]
			}
		} as any)

		const store = usePipelineStore()
		await store.fetchHostsForCluster('http://c1.test:8080')

		expect(store.availableHosts.length).toBe(2)
		expect(store.availableHosts[0].id).toBe('host-1')
		expect(store.availableHosts[0].name).toBe('Host Alpha')
	})

	it('fetches sinks dynamically for a cluster using sinksApi', async () => {
		vi.spyOn(sinksApi, 'getSinks').mockResolvedValue({
			success: true,
			data: {
				sinks: [
					{ sink_id: 'sink-1', name: 'Kafka Sink', type: 'kafka' },
					{ sink_id: 'sink-2', name: 'ClickHouse Sink', type: 'clickhouse' }
				]
			}
		} as any)

		const store = usePipelineStore()
		await store.fetchSinksForCluster('http://c1.test:8080')

		expect(store.availableSinks.length).toBe(2)
		expect(store.availableSinks[0].id).toBe('sink-1')
		expect(store.availableSinks[0].name).toBe('Kafka Sink')
	})

	it('creates a pipeline with cluster header and payload', async () => {
		const createSpy = vi.spyOn(pipelineManagementApi, 'createPipeline').mockResolvedValue({
			success: true,
			data: {
				pipeline_management_id: 'new-pipe-id'
			}
		} as any)

		const fetchSpy = vi.spyOn(pipelineManagementApi, 'getPipelines').mockResolvedValue({
			success: true,
			data: { pipeline_managements: [], total: 0 }
		} as any)

		const store = usePipelineStore()
		store.form.name = 'Test Pipeline'
		store.form.cluster_url = 'http://c1.test:8080'
		store.form.access = 'public'
		store.form.assignedHosts = ['host-1']
		store.form.assignedSinks = ['sink-1']

		const result = await store.savePipeline()

		expect(result).toBe(true)
		expect(createSpy).toHaveBeenCalledWith(
			{
				name: 'Test Pipeline',
				adm_status: 1,
				visibility: 1,
				host_ids: ['host-1'],
				sink_ids: ['sink-1']
			},
			['http://c1.test:8080']
		)
		expect(fetchSpy).toHaveBeenCalled()
	})

	it('deletes a pipeline using deletePipeline API', async () => {
		const deleteSpy = vi.spyOn(pipelineManagementApi, 'deletePipeline').mockResolvedValue({
			success: true
		} as any)

		const store = usePipelineStore()
		store.pipelines = [
			{
				id: 'pipe-to-delete',
				name: 'Pipeline To Delete',
				code: 'pipe-del-01',
				status: 'running',
				access: 'public',
				assignedHostsCount: 1,
				assignedSinksCount: 1,
				lastUpdate: 'now',
				assignedHosts: [],
				assignedSinks: [],
				cluster_url: 'http://c1.test:8080'
			}
		]

		const res = await store.deletePipeline('pipe-to-delete', 'http://c1.test:8080')
		expect(res).toBe(true)
		expect(deleteSpy).toHaveBeenCalledWith('pipe-to-delete', ['http://c1.test:8080'])
		expect(store.pipelines.length).toBe(0)
	})

	it('syncs and locks global cluster filter on entering pipeline builder, and resets on leaving', async () => {
		const AppNavbar = (await import('@/components/app/AppNavbar.vue')).default
		const appStore = useAppStore()
		appStore.activeClusterURL = null
		mockRoute.path = '/pipeline'
		mockRoute.query = {}

		const wrapper = mount(AppNavbar, {
			global: {
				stubs: {
					RouterLink: {
						template: '<a :href="to"><slot /></a>',
						props: ['to']
					},
					AppInputCluster: {
						name: 'AppInputCluster',
						props: ['modelValue', 'disabled', 'clearable'],
						template:
							'<div class="cluster-input-stub" :data-disabled="disabled" :data-clearable="clearable" :data-value="modelValue" />'
					},
					'b-navbar': {
						template:
							'<div><slot name="navbar-left"/><slot name="navbar-center"/><slot name="navbar-right"/></div>'
					},
					'b-navbar-item': { template: '<div><slot /></div>' },
					'b-switch': true,
					'b-dropdown': { template: '<div><slot /></div>' },
					'b-dropdown-toggle': { template: '<div><slot /></div>' },
					'b-dropdown-content': { template: '<div><slot /></div>' },
					'b-avatar': true,
					'b-icon': true,
					'b-text': true,
					'b-divider': true,
					'b-button': true
				}
			}
		})

		await flushPromises()

		let clusterStub = wrapper.find('.cluster-input-stub')
		expect(clusterStub.attributes('data-disabled')).toBe('false')
		expect(clusterStub.attributes('data-clearable')).toBe('true')
		expect(appStore.activeClusterURL).toBeNull()

		// Enter pipeline -> builder for Cluster 2
		mockRoute.query = { cluster_url: 'http://c2.test:8080' }
		mockRoute.path = '/pipeline/pipe-123/builder/canvas-456'
		await flushPromises()

		clusterStub = wrapper.find('.cluster-input-stub')
		expect(appStore.activeClusterURL).toBe('http://c2.test:8080')
		expect(clusterStub.attributes('data-disabled')).toBe('true')
		expect(clusterStub.attributes('data-clearable')).toBe('false')

		// Leave pipeline -> builder back to /pipeline
		mockRoute.query = {}
		mockRoute.path = '/pipeline'
		await flushPromises()

		clusterStub = wrapper.find('.cluster-input-stub')
		expect(appStore.activeClusterURL).toBeNull()
		expect(clusterStub.attributes('data-disabled')).toBe('false')
		expect(clusterStub.attributes('data-clearable')).toBe('true')
	})
})

// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { reactive } from 'vue'
import { mount, flushPromises } from '@vue/test-utils'
import { setActivePinia, createPinia } from 'pinia'
import { usePipelineStore } from '@/stores/pipelines.store'
import { useAppStore } from '@/stores/app.store'
import FormContent from '@/components/pipeline/form/FormContent.vue'
import pipelinesApi from '@/app/api/pipelines.api'
import hostsApi from '@/app/api/hosts.api'
import sinksApi from '@/app/api/sinks.api'
import appHelper from '@/app/helpers/app.helper'
import { FormMode } from '@@/shared/types/app.type'

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
		const getPipelinesSpy = vi.spyOn(pipelinesApi, 'getPipelines').mockResolvedValueOnce({
			success: true,
			data: {
				pipelines: [
					{
						pipeline_id: 'pipe-uuid-1',
						name: 'Pipeline One',
						adm_status: 1,
						visibility: 1,
						created_at: new Date().toISOString(),
						last_updated: new Date().toISOString(),
						cluster_url: 'http://c1.test:8080',
						cluster_name: 'Cluster 1'
					},
					{
						pipeline_id: 'pipe-uuid-2',
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

		const getPipelinesSpy = vi.spyOn(pipelinesApi, 'getPipelines').mockResolvedValue({
			success: true,
			data: {
				pipelines: [
					{
						pipeline_id: 'pipe-uuid-2',
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

	it('fetches sinks dynamically for a cluster using sinksApi', async () => {
		vi.spyOn(sinksApi, 'getSinks').mockResolvedValue({
			success: true,
			data: {
				sinks: [
					{ sink_id: 'sink-1', name: 'Kafka Sink', type: 'kafka' },
					{ sink_id: 'sink-2', name: 'ClickHouse Sink', type: 'clickhouse' }
				],
				total: 2
			}
		} as any)

		const store = usePipelineStore()
		await store.fetchSinksForCluster('http://c1.test:8080')

		expect(store.availableSinks.length).toBe(2)
		expect(store.availableSinksTotal).toBe(2)
		expect(store.availableSinks[0].id).toBe('sink-1')
		expect(store.availableSinks[0].name).toBe('Kafka Sink')
	})

	it('loads 100 hosts initially with actual total, supports loadMore, search, and cluster-scoped resolveAllHostIds', async () => {
		const makeHosts = (start: number, count: number) =>
			Array.from({ length: count }, (_, idx) => ({
				host_id: `host-${start + idx}`,
				name: `Host ${start + idx}`,
				ip: `10.0.0.${(start + idx) % 255}`
			}))

		const getHostsSpy = vi
			.spyOn(hostsApi, 'getHosts')
			.mockResolvedValueOnce({
				success: true,
				data: { hosts: makeHosts(0, 100), total: 1200 }
			} as any)
			.mockResolvedValueOnce({
				success: true,
				data: { hosts: makeHosts(100, 100), total: 1200 }
			} as any)
			.mockResolvedValueOnce({
				success: true,
				data: { hosts: makeHosts(0, 500), total: 1200 }
			} as any)
			.mockResolvedValueOnce({
				success: true,
				data: { hosts: makeHosts(500, 500), total: 1200 }
			} as any)
			.mockResolvedValueOnce({
				success: true,
				data: { hosts: makeHosts(1000, 200), total: 1200 }
			} as any)
			.mockResolvedValueOnce({
				success: true,
				data: { hosts: makeHosts(0, 1), total: 1 }
			} as any)

		const store = usePipelineStore()
		store.form.cluster_url = 'http://c2.test:8080'

		await store.fetchHostsForCluster()
		expect(getHostsSpy).toHaveBeenNthCalledWith(1, { limit: 100 }, ['http://c2.test:8080'])
		expect(store.availableHosts).toHaveLength(100)
		expect(store.availableHostsTotal).toBe(1200)

		await store.loadMoreHosts()
		expect(getHostsSpy).toHaveBeenNthCalledWith(2, { limit: 100, offset: 100 }, [
			'http://c2.test:8080'
		])
		expect(store.availableHosts).toHaveLength(200)

		const allIds = await store.resolveAllHostIds()
		expect(allIds).toHaveLength(1200)
		expect(getHostsSpy).toHaveBeenNthCalledWith(3, { limit: 500 }, ['http://c2.test:8080'])
		expect(getHostsSpy).toHaveBeenNthCalledWith(4, { limit: 500, offset: 500 }, [
			'http://c2.test:8080'
		])
		expect(getHostsSpy).toHaveBeenNthCalledWith(5, { limit: 500, offset: 1000 }, [
			'http://c2.test:8080'
		])

		await store.fetchHostsForCluster(undefined, 'Host 0')
		expect(getHostsSpy).toHaveBeenNthCalledWith(6, { limit: 100, name: 'Host 0' }, [
			'http://c2.test:8080'
		])
		expect(store.availableHosts).toHaveLength(1)
		expect(store.availableHostsTotal).toBe(1)
	})

	it('ignores stale host responses when switching clusters', async () => {
		let resolveFirst!: (val: any) => void
		const firstPromise = new Promise((r) => {
			resolveFirst = r
		})

		vi.spyOn(hostsApi, 'getHosts')
			.mockReturnValueOnce(firstPromise as any)
			.mockResolvedValueOnce({
				success: true,
				data: {
					hosts: [{ host_id: 'c2-host', name: 'Cluster 2 Host', ip: '10.2.0.1' }],
					total: 1
				}
			} as any)

		const store = usePipelineStore()
		const p1 = store.fetchHostsForCluster('http://c1.test:8080')
		const p2 = store.fetchHostsForCluster('http://c2.test:8080')
		await p2

		resolveFirst({
			success: true,
			data: {
				hosts: [{ host_id: 'c1-host', name: 'Cluster 1 Host', ip: '10.1.0.1' }],
				total: 1
			}
		})
		await p1

		expect(store.availableHosts).toHaveLength(1)
		expect(store.availableHosts[0].id).toBe('c2-host')
	})

	it('resets selector caches when toggling pipeline modal directly into UPDATE mode', async () => {
		vi.spyOn(hostsApi, 'getHosts')
			.mockResolvedValueOnce({
				success: true,
				data: {
					hosts: [{ host_id: 'old-host', name: 'Old Cluster Host', ip: '10.1.0.1' }],
					total: 1
				}
			} as any)
			.mockResolvedValueOnce({
				success: true,
				data: {
					hosts: [{ host_id: 'new-host', name: 'New Cluster Host', ip: '10.2.0.1' }],
					total: 1
				}
			} as any)
		vi.spyOn(sinksApi, 'getSinks').mockResolvedValue({
			success: true,
			data: { sinks: [], total: 0 }
		} as any)
		vi.spyOn(pipelinesApi, 'getPipelineById').mockResolvedValueOnce({
			success: true,
			data: {
				pipeline_id: 'pipe-2',
				name: 'Pipe 2',
				visibility: 1,
				adm_status: 1,
				host_ids: ['new-host'],
				sink_ids: [],
				last_updated: '2026-11-01T00:00:00Z'
			}
		} as any)

		const store = usePipelineStore()
		await store.fetchHostsForCluster('http://c1.test:8080')
		expect(store.hostNameCache['old-host']).toBe('Old Cluster Host')

		await store.togglePipelineModalForm(true, FormMode.UPDATE, {
			id: 'pipe-2',
			name: 'Pipe 2',
			cluster_url: 'http://c2.test:8080',
			access: 'public',
			assignedHosts: [],
			assignedSinks: []
		} as any)

		expect(store.hostNameCache['old-host']).toBeUndefined()
		expect(store.hostNameCache['new-host']).toBe('New Cluster Host')
	})

	it('hydrates missing visible assigned host and sink names (capped to 50) in UPDATE mode', async () => {
		const assignedHostIds = Array.from({ length: 60 }, (_, idx) => `host-${idx}`)
		const assignedSinkIds = ['sink-200']

		const getHostsSpy = vi
			.spyOn(hostsApi, 'getHosts')
			.mockResolvedValueOnce({
				success: true,
				data: {
					hosts: [{ host_id: 'host-0', name: 'Host 0', ip: '10.0.0.0' }],
					total: 300
				}
			} as any)
			.mockResolvedValueOnce({
				success: true,
				data: {
					hosts: assignedHostIds.slice(1, 50).map((id) => ({
						host_id: id,
						name: `Resolved ${id}`,
						ip: '10.0.1.1'
					})),
					total: 49
				}
			} as any)

		const getSinksSpy = vi
			.spyOn(sinksApi, 'getSinks')
			.mockResolvedValueOnce({
				success: true,
				data: {
					sinks: [{ sink_id: 'sink-0', name: 'Sink 0', type: 'kafka' }],
					total: 250
				}
			} as any)
			.mockResolvedValueOnce({
				success: true,
				data: {
					sinks: [{ sink_id: 'sink-200', name: 'Sink 200', type: 'clickhouse' }],
					total: 1
				}
			} as any)

		vi.spyOn(pipelinesApi, 'getPipelineById').mockResolvedValueOnce({
			success: true,
			data: {
				pipeline_id: 'pipe-edit-hydrate',
				name: 'Pipe Edit Hydrate',
				visibility: 1,
				adm_status: 1,
				host_ids: assignedHostIds,
				sink_ids: assignedSinkIds,
				last_updated: '2026-11-01T00:00:00Z'
			}
		} as any)

		const store = usePipelineStore()
		await store.togglePipelineModalForm(true, FormMode.UPDATE, {
			id: 'pipe-edit-hydrate',
			name: 'Pipe Edit Hydrate',
			cluster_url: 'http://c2.test:8080',
			access: 'public',
			assignedHosts: [],
			assignedSinks: []
		} as any)

		expect(getHostsSpy).toHaveBeenNthCalledWith(
			2,
			{ host_ids: assignedHostIds.slice(1, 50), limit: 49 },
			['http://c2.test:8080']
		)
		expect(getSinksSpy).toHaveBeenNthCalledWith(2, { sink_ids: ['sink-200'], limit: 1 }, [
			'http://c2.test:8080'
		])
		expect(store.hostNameCache['host-0']).toBe('Host 0')
		expect(store.hostNameCache['host-49']).toBe('Resolved host-49')
		expect(store.hostNameCache['host-50']).toBeUndefined()
		expect(store.sinkNameCache['sink-200']).toBe('Sink 200')
	})

	it('hydrates newly exposed visible host tag when removing a tag from >50 assigned hosts in FormContent', async () => {
		vi.useFakeTimers()
		try {
			const assignedHostIds = Array.from({ length: 52 }, (_, idx) => `host-${idx}`)
			const getHostsSpy = vi
				.spyOn(hostsApi, 'getHosts')
				.mockResolvedValueOnce({
					success: true,
					data: {
						hosts: assignedHostIds.slice(0, 50).map((id) => ({
							host_id: id,
							name: `Host ${id}`,
							ip: '10.0.0.1'
						})),
						total: 100
					}
				} as any)
				.mockResolvedValueOnce({
					success: true,
					data: {
						hosts: [{ host_id: 'host-50', name: 'Resolved Host 50', ip: '10.0.0.50' }],
						total: 1
					}
				} as any)

			const store = usePipelineStore()
			store.form.cluster_url = 'http://c2.test:8080'
			store.form.assignedHosts = [...assignedHostIds]
			await store.fetchHostsForCluster('http://c2.test:8080')

			const wrapper = mount(FormContent, {
				global: {
					stubs: {
						AppInputCluster: true,
						AppInputText: true,
						HostSelectorPopover: true,
						SinkSelectorPopover: true,
						UiIcon: true
					}
				}
			})

			// Remove the first visible host tag -> host-50 shifts into the visible 50-tag window
			store.form.assignedHosts.splice(0, 1)
			await vi.advanceTimersByTimeAsync(200)
			await flushPromises()

			expect(getHostsSpy).toHaveBeenNthCalledWith(2, { host_ids: ['host-50'], limit: 1 }, [
				'http://c2.test:8080'
			])
			expect(store.hostNameCache['host-50']).toBe('Resolved Host 50')
			expect(wrapper.text()).toContain('Resolved Host 50')
			wrapper.unmount()
		} finally {
			vi.useRealTimers()
		}
	})

	it('throws on resolveAllHostIds/resolveAllSinkIds and clears lists when API returns undefined or success false', async () => {
		vi.spyOn(hostsApi, 'getHosts')
			.mockResolvedValueOnce({
				success: true,
				data: {
					hosts: [{ host_id: 'host-1', name: 'Host 1', ip: '10.0.0.1' }],
					total: 1
				}
			} as any)
			.mockResolvedValueOnce(undefined)
			.mockResolvedValueOnce({ success: false, message: 'Hosts unavailable' } as any)
		vi.spyOn(sinksApi, 'getSinks')
			.mockResolvedValueOnce(undefined)
			.mockResolvedValueOnce({ success: false, message: 'Sinks unavailable' } as any)

		const store = usePipelineStore()
		await store.fetchHostsForCluster('http://c1.test:8080')
		expect(store.availableHosts).toHaveLength(1)

		await store.fetchHostsForCluster('http://c1.test:8080')
		expect(store.availableHosts).toEqual([])
		expect(store.availableHostsTotal).toBe(0)

		await expect(store.resolveAllHostIds('http://c1.test:8080')).rejects.toThrow(
			'Hosts unavailable'
		)
		await expect(store.resolveAllSinkIds('http://c1.test:8080')).rejects.toThrow(
			'Failed to load sinks.'
		)
		await expect(store.resolveAllSinkIds('http://c1.test:8080')).rejects.toThrow(
			'Sinks unavailable'
		)
	})

	it('creates a pipeline with cluster header and payload', async () => {
		const createSpy = vi.spyOn(pipelinesApi, 'createPipeline').mockResolvedValue({
			success: true,
			data: {
				pipeline_id: 'new-pipe-id'
			}
		} as any)

		const fetchSpy = vi.spyOn(pipelinesApi, 'getPipelines').mockResolvedValue({
			success: true,
			data: { pipelines: [], total: 0 }
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
		const deleteSpy = vi.spyOn(pipelinesApi, 'deletePipeline').mockResolvedValue({
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

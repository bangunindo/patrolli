// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { reactive } from 'vue'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import CanvasTopNav from '@/components/pipeline/builder/CanvasTopNav.vue'
import EdgeItemsModal from '@/components/pipeline/builder/modals/EdgeItemsModal.vue'
import BuilderSinkNode from '@/components/pipeline/builder/nodes/BuilderSinkNode.vue'
import HostSelectorPopover from '@/components/pipeline/HostSelectorPopover.vue'
import SinkSelectorPopover from '@/components/pipeline/SinkSelectorPopover.vue'
import { usePipelineBuildStore } from '@/stores/pipeline.builder.store'
import pipelinesApi from '@/app/api/pipelines.api'
import hostsApi from '@/app/api/hosts.api'
import sinksApi from '@/app/api/sinks.api'
import itemsApi from '@/app/api/items.api'

vi.mock('vue-router', () => ({
	useRouter: () => ({
		push: vi.fn()
	})
}))

describe('Pipeline Builder Existing Host & Sink Selection', () => {
	beforeEach(() => {
		setActivePinia(createPinia())
		vi.clearAllMocks()
	})

	function seedStore() {
		const store = usePipelineBuildStore()
		store.pipelineId = 'pipe-1'
		store.canvasId = 'canvas-1'
		store.pipelineDetail = {
			pipeline_id: 'pipe-1',
			name: 'Telemetry Pipeline',
			visibility: 1,
			adm_status: 1,
			created_at: '2026-11-01T00:00:00Z',
			last_updated: '2026-11-01T00:00:00Z',
			host_ids: ['host-1'],
			sink_ids: ['sink-1'],
			canvases: [{ canvas_id: 'canvas-1', last_updated: '2026-11-01T00:00:00Z' }]
		} as any
		store.availableHosts = [
			{ host_id: 'host-1', name: 'Router Core', ip: '10.0.0.1' },
			{ host_id: 'host-2', name: 'Firewall Edge', ip: '10.0.0.2' }
		]
		store.availableSinks = [
			{ sink_id: 'sink-1', name: 'Kafka Primary', type: 'kafka' },
			{ sink_id: 'sink-2', name: 'ClickHouse Archive', type: 'clickhouse' }
		]
		store.pipelineItems = [
			{ item_id: 'item-1', host_id: 'host-1', name: 'CPU Load' },
			{ item_id: 'item-2', host_id: 'host-2', name: 'Packet Drop' }
		]
		return store
	}

	it('renders Select Hosts and Select Sinks buttons in CanvasTopNav without create forms', () => {
		seedStore()

		const wrapper = mount(CanvasTopNav, {
			global: {
				stubs: {
					UiButton: { template: '<button @click="$emit(\'click\')"><slot /></button>' },
					UiIcon: { template: '<i />' },
					HostSelectorPopover: { template: '<div class="host-selector-stub" />' },
					SinkSelectorPopover: { template: '<div class="sink-selector-stub" />' },
					ModalConfirmationDelete: { template: '<div class="confirm-stub" />' }
				}
			}
		})

		expect(wrapper.text()).toContain('Select Hosts')
		expect(wrapper.text()).toContain('Select Sinks')
		expect(wrapper.text()).not.toContain('Add Host')
		expect(wrapper.text()).not.toContain('Add Sink')
	})

	const selectorMountGlobal = {
		stubs: {
			UiIcon: { template: '<i />' },
			DialogPortal: { template: '<div><slot /></div>' },
			Teleport: { template: '<div><slot /></div>' }
		}
	}

	it('enforces minSelection in HostSelectorPopover and emits selected IDs on apply', async () => {
		const wrapper = mount(HostSelectorPopover, {
			attachTo: document.body,
			props: {
				modelValue: true,
				inline: false,
				hosts: [
					{ id: 'host-1', name: 'Router Core', ip: '10.0.0.1', environment: 'prod' },
					{ id: 'host-2', name: 'Firewall Edge', ip: '10.0.0.2', environment: 'prod' }
				],
				selectedIds: ['host-1'],
				minSelection: 1
			},
			global: selectorMountGlobal
		})

		const clearBtn = wrapper.findAll('button').find((b) => b.text() === 'Clear All')!
		await clearBtn.trigger('click')

		const applyBtn = wrapper
			.findAll('button')
			.find((b) => b.text().includes('Apply Selection'))!
		await applyBtn.trigger('click')

		expect(wrapper.find('[role="alert"]').text()).toContain(
			'At least one host must be selected.'
		)
		expect(wrapper.emitted('apply')).toBeUndefined()

		const hostRows = wrapper.findAll('.cursor-pointer')
		await hostRows[1]!.trigger('click')
		await applyBtn.trigger('click')

		expect(wrapper.emitted('apply')?.[0]).toEqual([['host-2']])
		wrapper.unmount()
	})

	it('emits selected sink IDs from SinkSelectorPopover and toggles checkbox once', async () => {
		const wrapper = mount(SinkSelectorPopover, {
			attachTo: document.body,
			props: {
				modelValue: true,
				inline: false,
				sinks: [
					{ id: 'sink-1', name: 'Kafka Primary', type: 'kafka' },
					{ id: 'sink-2', name: 'ClickHouse Archive', type: 'clickhouse' }
				],
				selectedIds: ['sink-1']
			},
			global: selectorMountGlobal
		})

		const sinkRows = wrapper.findAll('.cursor-pointer')
		await sinkRows[1]!.trigger('click')

		const applyBtn = wrapper
			.findAll('button')
			.find((b) => b.text().includes('Apply Selection'))!
		await applyBtn.trigger('click')

		expect(wrapper.emitted('apply')?.[0]).toEqual([['sink-1', 'sink-2']])

		const checkboxes = wrapper.findAll('button[role="checkbox"]')
		await checkboxes[0]!.trigger('click')
		await applyBtn.trigger('click')
		expect(wrapper.emitted('apply')?.[1]).toEqual([['sink-2']])
		wrapper.unmount()
	})

	it('refreshes cluster hosts/sinks when opening builder selector modals', async () => {
		const store = seedStore()
		const getHostsSpy = vi.spyOn(hostsApi, 'getHosts').mockResolvedValue({
			success: true,
			data: { hosts: [{ host_id: 'host-3', name: 'New Cluster Host', ip: '10.0.0.3' }] }
		} as any)
		const getSinksSpy = vi.spyOn(sinksApi, 'getSinks').mockResolvedValue({
			success: true,
			data: { sinks: [{ sink_id: 'sink-3', name: 'New Cluster Sink', type: 'file' }] }
		} as any)

		await store.openHostSelectorModal()
		expect(store.hostSelectorModalOpen).toBe(true)
		expect(getHostsSpy).toHaveBeenCalledTimes(1)
		expect(store.availableHosts).toHaveLength(1)

		await store.openSinkSelectorModal()
		expect(store.sinkSelectorModalOpen).toBe(true)
		expect(getSinksSpy).toHaveBeenCalledTimes(1)
		expect(store.availableSinks).toHaveLength(1)
	})

	it('assigns existing hosts immediately via updatePipeline and syncs SourceNode & items', async () => {
		const store = seedStore()
		store.nodes = [
			{
				id: 'source-1',
				node_id: 'source-1',
				type: 'source',
				position: { x: 80, y: 160 },
				data: {
					id: 'source-1',
					hostCount: 1,
					itemCount: 1,
					hosts: ['Router Core'],
					items: ['item-1'],
					health: { healthy: 1, warning: 0, error: 0 }
				}
			}
		]
		store.hostSelectorModalOpen = true

		const updateSpy = vi.spyOn(pipelinesApi, 'updatePipeline').mockResolvedValue({
			success: true,
			data: {
				...store.pipelineDetail,
				host_ids: ['host-1', 'host-2'],
				last_updated: '2026-11-01T01:00:00Z'
			}
		} as any)
		const getItemsSpy = vi.spyOn(itemsApi, 'getItems').mockResolvedValue({
			success: true,
			data: {
				items: [
					{ item_id: 'item-1', host_id: 'host-1', name: 'CPU Load' },
					{ item_id: 'item-2', host_id: 'host-2', name: 'Packet Drop' }
				]
			}
		} as any)

		const ok = await store.applyPipelineHosts(['host-1', 'host-2'])

		expect(ok).toBe(true)
		expect(updateSpy).toHaveBeenCalledWith(
			'pipe-1',
			expect.objectContaining({
				host_ids: ['host-1', 'host-2'],
				sink_ids: ['sink-1'],
				force: false
			}),
			undefined,
			{ suppressErrorStatuses: [409] }
		)
		expect(getItemsSpy).toHaveBeenCalled()
		expect(store.hostSelectorModalOpen).toBe(false)
		expect(store.pipelineDetail?.host_ids).toEqual(['host-1', 'host-2'])
		expect(store.nodes[0].data.hostCount).toBe(2)
		expect(store.nodes[0].data.hosts).toEqual(['Router Core', 'Firewall Edge'])
		expect(store.isDirty).toBe(false)
	})

	it('prompts for confirmation when dropping an in-use sink and forces removal on confirm', async () => {
		const store = seedStore()
		store.nodes = [
			{
				id: 'sink-node-1',
				node_id: 'sink-node-1',
				type: 'sink',
				position: { x: 600, y: 160 },
				data: {
					id: 'sink-node-1',
					sinkId: 'sink-1',
					sinkName: 'Kafka Primary',
					type: 'kafka'
				}
			}
		]
		store.sinkSelectorModalOpen = true

		const updateSpy = vi.spyOn(pipelinesApi, 'updatePipeline').mockResolvedValue({
			success: true,
			data: {
				...store.pipelineDetail,
				sink_ids: [],
				last_updated: '2026-11-01T02:00:00Z'
			}
		} as any)

		const initialOk = await store.applyPipelineSinks([])
		expect(initialOk).toBe(false)
		expect(updateSpy).not.toHaveBeenCalled()
		expect(store.subsetConfirmation.isOpen).toBe(true)
		expect(store.subsetConfirmation.message).toContain('1 sink node(s)')

		const confirmOk = await store.confirmSubsetForceUpdate()
		expect(confirmOk).toBe(true)
		expect(updateSpy).toHaveBeenCalledWith(
			'pipe-1',
			expect.objectContaining({
				sink_ids: [],
				force: true
			}),
			undefined,
			undefined
		)
		expect(store.subsetConfirmation.isOpen).toBe(false)
		expect(store.sinkSelectorModalOpen).toBe(false)
		expect(store.nodes[0].data.sinkId).toBeUndefined()
		expect(store.nodes[0].data.sinkName).toBeUndefined()
		expect(store.nodes[0].data.type).toBe('No sink selected')
	})

	it('handles backend 409 Subset in use fallback for dropped hosts and cleans source items/edges on confirm', async () => {
		const store = seedStore()
		store.nodes = [
			{
				id: 'source-1',
				node_id: 'source-1',
				type: 'source',
				position: { x: 80, y: 160 },
				data: {
					id: 'source-1',
					hostCount: 1,
					itemCount: 1,
					item_count: 1,
					hosts: ['Router Core'],
					items: ['item-1'],
					health: { healthy: 1, warning: 0, error: 0 }
				}
			},
			{
				id: 'proc-1',
				node_id: 'proc-1',
				type: 'processor',
				position: { x: 350, y: 160 },
				data: { id: 'proc-1', name: 'Group', itemCount: 1, processors: [] }
			}
		]
		store.edges = [
			{
				id: 'edge-1',
				source: 'source-1',
				target: 'proc-1',
				data: { selectedItemIds: ['item-1'], label: '1 Items' }
			}
		]
		// Clear local items to force hitting the backend 409 fallback branch
		store.pipelineDetail!.host_ids = ['host-1', 'host-2']
		store.pipelineItems = [{ item_id: 'item-1', host_id: 'host-1', name: 'CPU Load' }]
		store.nodes[0].data.items = []
		store.edges[0].data.selectedItemIds = []

		const updateSpy = vi
			.spyOn(pipelinesApi, 'updatePipeline')
			.mockRejectedValueOnce({
				status: 409,
				data: {
					error: {
						title: 'Subset in use',
						data: { node_ids: ['source-1'], item_ids: ['item-1'], item_count: 1 }
					}
				}
			})
			.mockResolvedValueOnce({
				success: true,
				data: {
					...store.pipelineDetail,
					host_ids: ['host-2'],
					last_updated: '2026-11-01T03:00:00Z'
				}
			} as any)

		vi.spyOn(itemsApi, 'getItems').mockResolvedValue({
			success: true,
			data: { items: [{ item_id: 'item-2', host_id: 'host-2', name: 'Packet Drop' }] }
		} as any)

		// Put item-1 back on node/edge before confirming force
		const okFirst = await store.applyPipelineHosts(['host-2'])
		expect(okFirst).toBe(false)
		expect(store.subsetConfirmation.isOpen).toBe(true)
		expect(store.subsetConfirmation.message).toContain('1 item(s)')

		store.nodes[0].data.items = ['item-1']
		store.edges[0].data.selectedItemIds = ['item-1']

		const okConfirm = await store.confirmSubsetForceUpdate()
		expect(okConfirm).toBe(true)
		expect(updateSpy).toHaveBeenCalledTimes(2)
		expect(store.nodes[0].data.items).toEqual([])
		expect(store.nodes[0].data.itemCount).toBe(0)
		expect(store.edges[0].data.selectedItemIds).toEqual([])
		expect(store.edges[0].data.label).toBe('0 Items')
		expect(store.nodes[1].data.itemCount).toBe(0)
	})

	it('disables Select Hosts and Select Sinks when pipelineDetail is missing or canvas is loading', () => {
		const store = usePipelineBuildStore()
		store.pipelineDetail = null
		store.loading.canvas = false

		const mountTopNav = () =>
			mount(CanvasTopNav, {
				global: {
					stubs: {
						UiButton: {
							props: ['disabled'],
							template:
								'<button :disabled="disabled" @click="$emit(\'click\')"><slot /></button>'
						},
						UiIcon: { template: '<i />' },
						HostSelectorPopover: { template: '<div />' },
						SinkSelectorPopover: { template: '<div />' },
						ModalConfirmationDelete: { template: '<div />' }
					}
				}
			})

		const wrapperMissing = mountTopNav()
		const selectHostsMissing = wrapperMissing
			.findAll('button')
			.find((b) => b.text().includes('Select Hosts'))!
		const selectSinksMissing = wrapperMissing
			.findAll('button')
			.find((b) => b.text().includes('Select Sinks'))!
		expect(selectHostsMissing.attributes('disabled')).toBeDefined()
		expect(selectSinksMissing.attributes('disabled')).toBeDefined()

		seedStore()
		store.loading.canvas = true
		const wrapperLoading = mountTopNav()
		const selectHostsLoading = wrapperLoading
			.findAll('button')
			.find((b) => b.text().includes('Select Hosts'))!
		expect(selectHostsLoading.attributes('disabled')).toBeDefined()
	})

	it('sets selectorError when applying hosts/sinks before pipelineDetail is loaded', async () => {
		const store = usePipelineBuildStore()
		store.pipelineId = 'pipe-1'
		store.pipelineDetail = null

		const ok = await store.applyPipelineHosts(['host-1'])
		expect(ok).toBe(false)
		expect(store.selectorError).toBe('Pipeline details are not loaded yet.')
	})

	it('emits clearError on interaction and renders UiAlert for errors', async () => {
		const hostWrapper = mount(HostSelectorPopover, {
			attachTo: document.body,
			props: {
				modelValue: true,
				inline: false,
				hosts: [{ id: 'host-1', name: 'Router Core', ip: '10.0.0.1', environment: 'prod' }],
				selectedIds: ['host-1'],
				errorMessage: 'Server error'
			},
			global: selectorMountGlobal
		})

		expect(hostWrapper.find('[role="alert"]').text()).toContain('Server error')
		await hostWrapper.find('.cursor-pointer').trigger('click')
		expect(hostWrapper.emitted('clearError')).toHaveLength(1)
		hostWrapper.unmount()
	})

	it('loads initial 100 hosts and sinks with actual total, supports loadMore and resolveAllIds', async () => {
		const store = seedStore()
		const makePage = (prefix: string, start: number, count: number) =>
			Array.from({ length: count }, (_, idx) => ({
				[`${prefix}_id`]: `${prefix}-${start + idx}`,
				name: `${prefix}-${start + idx}`
			}))

		const getHostsSpy = vi
			.spyOn(hostsApi, 'getHosts')
			.mockResolvedValueOnce({
				success: true,
				data: { hosts: makePage('host', 0, 100), total: 120 }
			} as any)
			.mockResolvedValueOnce({
				success: true,
				data: { hosts: makePage('host', 100, 20), total: 120 }
			} as any)
			.mockResolvedValueOnce({
				success: true,
				data: { hosts: makePage('host', 0, 120), total: 120 }
			} as any)

		const getSinksSpy = vi
			.spyOn(sinksApi, 'getSinks')
			.mockResolvedValueOnce({
				success: true,
				data: { sinks: makePage('sink', 0, 100), total: 105 }
			} as any)
			.mockResolvedValueOnce({
				success: true,
				data: { sinks: makePage('sink', 100, 5), total: 105 }
			} as any)
			.mockResolvedValueOnce({
				success: true,
				data: { sinks: makePage('sink', 0, 105), total: 105 }
			} as any)

		await store.openHostSelectorModal()
		expect(getHostsSpy).toHaveBeenNthCalledWith(1, { limit: 100 }, undefined)
		expect(store.availableHosts).toHaveLength(100)
		expect(store.availableHostsTotal).toBe(120)

		await store.loadMoreSelectorHosts()
		expect(getHostsSpy).toHaveBeenNthCalledWith(2, { limit: 100, offset: 100 }, undefined)
		expect(store.availableHosts).toHaveLength(120)

		const resolvedHostIds = await store.resolveAllSelectorHostIds()
		expect(getHostsSpy).toHaveBeenNthCalledWith(3, { limit: 500 }, undefined)
		expect(resolvedHostIds).toHaveLength(120)

		await store.openSinkSelectorModal()
		expect(getSinksSpy).toHaveBeenNthCalledWith(1, { limit: 100 }, undefined)
		expect(store.availableSinks).toHaveLength(100)
		expect(store.availableSinksTotal).toBe(105)

		await store.loadMoreSelectorSinks()
		expect(getSinksSpy).toHaveBeenNthCalledWith(2, { limit: 100, offset: 100 }, undefined)
		expect(store.availableSinks).toHaveLength(105)

		const resolvedSinkIds = await store.resolveAllSelectorSinkIds()
		expect(getSinksSpy).toHaveBeenNthCalledWith(3, { limit: 500 }, undefined)
		expect(resolvedSinkIds).toHaveLength(105)
	})

	it('refreshes backend Source node via getPipelineNode after forced host removal when items exceed page size (500)', async () => {
		const store = seedStore()
		store.pipelineDetail!.host_ids = ['host-1', 'host-2']
		store.canvasGraph = {
			pipeline_id: 'pipe-1',
			canvas_id: 'canvas-1',
			last_updated: '2026-11-01T00:00:00Z',
			graph: {
				nodes: [
					{
						node_id: 'source-1',
						kind: 'source',
						position_x: 80,
						position_y: 160,
						data: { item_count: 150 }
					}
				],
				relations: []
			}
		} as any

		const page1RemainingItems = Array.from({ length: 500 }, (_, idx) => ({
			item_id: `item-h2-${idx}`,
			host_id: 'host-2',
			name: `Metric ${idx}`
		}))
		const page2RemainingItems = Array.from({ length: 10 }, (_, idx) => ({
			item_id: `item-h2-${500 + idx}`,
			host_id: 'host-2',
			name: `Metric ${500 + idx}`
		}))

		store.nodes = [
			{
				id: 'source-1',
				node_id: 'source-1',
				type: 'source',
				position: { x: 80, y: 160 },
				data: {
					id: 'source-1',
					hostCount: 2,
					itemCount: 150,
					item_count: 150,
					hosts: ['Router Core', 'Firewall Edge'],
					items: ['item-1'],
					health: { healthy: 2, warning: 0, error: 0 }
				}
			},
			{
				id: 'proc-1',
				node_id: 'proc-1',
				type: 'processor',
				position: { x: 350, y: 160 },
				data: { id: 'proc-1', name: 'Group', itemCount: 150, processors: [] }
			}
		]
		store.edges = [
			{
				id: 'edge-1',
				source: 'source-1',
				target: 'proc-1',
				data: { selectedItemIds: ['item-1'], label: '150 Items' }
			}
		]

		vi.spyOn(pipelinesApi, 'updatePipeline').mockResolvedValue({
			success: true,
			data: {
				...store.pipelineDetail,
				host_ids: ['host-2'],
				last_updated: '2026-11-01T04:00:00Z',
				canvases: [{ canvas_id: 'canvas-1', last_updated: '2026-11-01T04:00:00Z' }]
			}
		} as any)
		vi.spyOn(itemsApi, 'getItems')
			.mockResolvedValueOnce({
				success: true,
				data: { items: page1RemainingItems, total: 510 }
			} as any)
			.mockResolvedValueOnce({
				success: true,
				data: { items: page2RemainingItems, total: 510 }
			} as any)
		const getNodeSpy = vi
			.spyOn(pipelinesApi, 'getPipelineNode')
			.mockResolvedValueOnce({
				success: true,
				data: {
					pipeline_id: 'pipe-1',
					canvas_id: 'canvas-1',
					node_id: 'source-1',
					last_updated: '2026-11-01T04:00:00Z',
					kind: 'source',
					data: {
						items: page1RemainingItems,
						total: 510
					}
				}
			} as any)
			.mockResolvedValueOnce({
				success: true,
				data: {
					pipeline_id: 'pipe-1',
					canvas_id: 'canvas-1',
					node_id: 'source-1',
					last_updated: '2026-11-01T04:00:00Z',
					kind: 'source',
					data: {
						items: page2RemainingItems,
						total: 510
					}
				}
			} as any)

		const ok = await store.applyPipelineHosts(['host-2'])
		expect(ok).toBe(false)
		expect(store.subsetConfirmation.isOpen).toBe(true)

		const confirmed = await store.confirmSubsetForceUpdate()
		expect(confirmed).toBe(true)
		expect(getNodeSpy).toHaveBeenNthCalledWith(
			1,
			{
				pipeline_id: 'pipe-1',
				canvas_id: 'canvas-1',
				node_id: 'source-1',
				query: { limit: 500 }
			},
			undefined
		)
		expect(getNodeSpy).toHaveBeenNthCalledWith(
			2,
			{
				pipeline_id: 'pipe-1',
				canvas_id: 'canvas-1',
				node_id: 'source-1',
				query: { limit: 500, offset: 500 }
			},
			undefined
		)
		expect(store.canvasGraph?.last_updated).toBe('2026-11-01T04:00:00Z')
		expect(store.nodes[0].data.item_count).toBe(510)
		expect(store.nodes[0].data.itemCount).toBe(510)
		expect(store.nodes[0].data.items).toHaveLength(510)
		expect(store.edges[0].data.label).toBe('510 Items')
		expect(store.nodes[1].data.itemCount).toBe(510)
	})

	it('preserves delta-based itemCount fallback when unsaved Source node has total exceeding loaded items', async () => {
		const store = seedStore()
		store.pipelineDetail!.host_ids = ['host-1', 'host-2']
		store.canvasGraph = null
		store.nodes = [
			{
				id: 'source-unsaved',
				node_id: 'source-unsaved',
				type: 'source',
				position: { x: 80, y: 160 },
				data: {
					id: 'source-unsaved',
					hostCount: 2,
					itemCount: 150,
					item_count: 150,
					hosts: ['Router Core', 'Firewall Edge'],
					items: ['item-1', 'item-2'],
					health: { healthy: 2, warning: 0, error: 0 }
				}
			}
		]

		vi.spyOn(pipelinesApi, 'updatePipeline').mockResolvedValue({
			success: true,
			data: {
				...store.pipelineDetail,
				host_ids: ['host-2'],
				last_updated: '2026-11-01T04:00:00Z'
			}
		} as any)
		vi.spyOn(itemsApi, 'getItems').mockResolvedValue({
			success: true,
			data: { items: [{ item_id: 'item-2', host_id: 'host-2', name: 'Packet Drop' }] }
		} as any)

		await store.applyPipelineHosts(['host-2'])
		const confirmed = await store.confirmSubsetForceUpdate()

		expect(confirmed).toBe(true)
		expect(store.nodes[0].data.items).toEqual(['item-2'])
		expect(store.nodes[0].data.itemCount).toBe(149)
		expect(store.nodes[0].data.item_count).toBe(149)
	})

	it('skips PUT request and closes modal on no-op host and sink selection apply', async () => {
		const store = seedStore()
		const updateSpy = vi.spyOn(pipelinesApi, 'updatePipeline')

		store.hostSelectorModalOpen = true
		const hostOk = await store.applyPipelineHosts(['host-1'])
		expect(hostOk).toBe(true)
		expect(updateSpy).not.toHaveBeenCalled()
		expect(store.hostSelectorModalOpen).toBe(false)

		store.sinkSelectorModalOpen = true
		const sinkOk = await store.applyPipelineSinks(['sink-1'])
		expect(sinkOk).toBe(true)
		expect(updateSpy).not.toHaveBeenCalled()
		expect(store.sinkSelectorModalOpen).toBe(false)
	})

	it('sets selectorError when opening host or sink selector modal fails to fetch', async () => {
		const store = seedStore()
		vi.spyOn(hostsApi, 'getHosts').mockRejectedValueOnce(new Error('Network error'))
		vi.spyOn(sinksApi, 'getSinks').mockRejectedValueOnce(new Error('Network error'))

		await store.openHostSelectorModal()
		expect(store.hostSelectorModalOpen).toBe(true)
		expect(store.selectorError).toBe('Failed to load available hosts.')
		expect(store.loadingResources.hosts).toBe(false)

		await store.openSinkSelectorModal()
		expect(store.sinkSelectorModalOpen).toBe(true)
		expect(store.selectorError).toBe('Failed to load available sinks.')
		expect(store.loadingResources.sinks).toBe(false)
	})

	it('closes non-inline selector dialog via UiDialog Escape unless saving', async () => {
		const hostWrapper = mount(HostSelectorPopover, {
			attachTo: document.body,
			props: {
				modelValue: true,
				inline: false,
				hosts: [{ id: 'h1', name: 'Host 1', ip: '10.0.0.1', environment: 'prod' }],
				selectedIds: ['h1'],
				saving: false
			},
			global: selectorMountGlobal
		})
		await hostWrapper.setProps({ saving: true })
		document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
		expect(hostWrapper.emitted('close')).toBeUndefined()

		await hostWrapper.setProps({ saving: false })
		document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
		expect(hostWrapper.emitted('update:modelValue')).toEqual([[false]])
		expect(hostWrapper.emitted('close')).toHaveLength(1)
		hostWrapper.unmount()
	})

	it('suspends selector dialog during confirmation without closing or resetting draft and supports inline UiCard mode', async () => {
		const hostWrapper = mount(HostSelectorPopover, {
			attachTo: document.body,
			props: {
				modelValue: true,
				inline: false,
				suspended: false,
				hosts: [
					{ id: 'h1', name: 'Host 1', ip: '10.0.0.1', environment: 'prod' },
					{ id: 'h2', name: 'Host 2', ip: '10.0.0.2', environment: 'prod' }
				],
				selectedIds: ['h1']
			},
			global: selectorMountGlobal
		})

		await hostWrapper.findAll('.cursor-pointer')[1]!.trigger('click')
		expect(hostWrapper.text()).toContain('2 of 2 Selected')

		await hostWrapper.setProps({ suspended: true })
		const suspendedDialog = hostWrapper.find('[role="dialog"]')
		expect(
			!suspendedDialog.exists() || suspendedDialog.attributes('data-state') === 'closed'
		).toBe(true)
		expect(hostWrapper.emitted('close')).toBeUndefined()

		await hostWrapper.setProps({ suspended: false })
		expect(hostWrapper.find('[role="dialog"]').attributes('data-state')).toBe('open')
		expect(hostWrapper.text()).toContain('2 of 2 Selected')
		hostWrapper.unmount()

		const inlineSink = mount(SinkSelectorPopover, {
			props: {
				inline: true,
				sinks: [{ id: 's1', name: 'Sink 1', type: 'kafka' }],
				selectedIds: ['s1']
			},
			global: selectorMountGlobal
		})
		expect(inlineSink.find('[role="dialog"]').exists()).toBe(false)
		expect(inlineSink.text()).toContain('Select Sinks')
		expect(inlineSink.text()).toContain('Sink 1')
		inlineSink.unmount()
	})

	it('supports Select All toggle in SinkSelectorPopover and multi-page fetchPipelineItems', async () => {
		const sinkWrapper = mount(SinkSelectorPopover, {
			attachTo: document.body,
			props: {
				modelValue: true,
				inline: false,
				sinks: [
					{ id: 'sink-1', name: 'Kafka Primary', type: 'kafka' },
					{ id: 'sink-2', name: 'ClickHouse Archive', type: 'clickhouse' }
				],
				selectedIds: ['sink-1']
			},
			global: selectorMountGlobal
		})

		const selectAllBtn = sinkWrapper.findAll('button').find((b) => b.text() === 'Select All')!
		expect(selectAllBtn).toBeDefined()

		await selectAllBtn.trigger('click')
		expect(sinkWrapper.text()).toContain('2 of 2 Selected')

		await sinkWrapper.setProps({ saving: true })
		await selectAllBtn.trigger('click')
		await sinkWrapper.findAll('.cursor-pointer')[0]!.trigger('click')
		expect(sinkWrapper.text()).toContain('2 of 2 Selected')

		await sinkWrapper.setProps({ saving: false })
		await selectAllBtn.trigger('click')
		expect(sinkWrapper.text()).toContain('0 of 2 Selected')
		sinkWrapper.unmount()

		const store = seedStore()
		const makeItemPage = (start: number, count: number) =>
			Array.from({ length: count }, (_, idx) => ({
				item_id: `item-${start + idx}`,
				host_id: 'host-1',
				name: `Item ${start + idx}`
			}))
		const getItemsSpy = vi
			.spyOn(itemsApi, 'getItems')
			.mockResolvedValueOnce({
				success: true,
				data: { items: makeItemPage(0, 500), total: 530 }
			} as any)
			.mockResolvedValueOnce({
				success: true,
				data: { items: makeItemPage(500, 30), total: 530 }
			} as any)

		await store.fetchPipelineItems()
		expect(getItemsSpy).toHaveBeenNthCalledWith(
			1,
			{ host_ids: ['host-1'], limit: 500 },
			undefined
		)
		expect(getItemsSpy).toHaveBeenNthCalledWith(
			2,
			{ host_ids: ['host-1'], limit: 500, offset: 500 },
			undefined
		)
		expect(store.pipelineItems).toHaveLength(530)
	})

	it('syncs inline HostSelectorPopover draftSelected on in-place selectedIds splice without clearing search, detects item.host_id on Source node when pipelineItems is empty, and syncs canvasGraph timestamp on 409 refresh', async () => {
		const hosts = [
			{ id: 'h1', name: 'Router Core', ip: '10.0.0.1', environment: 'prod' as const },
			{ id: 'h2', name: 'Firewall Edge', ip: '10.0.0.2', environment: 'prod' as const }
		]
		const selectedIds = reactive(['h1', 'h2'])

		const inlineHost = mount(HostSelectorPopover, {
			props: {
				inline: true,
				hosts,
				selectedIds
			},
			global: selectorMountGlobal
		})
		expect(inlineHost.text()).toContain('2 of 2 Selected')

		const searchInput = inlineHost.find('input')
		await searchInput.setValue('Router')
		expect((searchInput.element as HTMLInputElement).value).toBe('Router')

		// Simulate clicking removeHost('h2') in FormContent.vue while inline card is open
		selectedIds.splice(1, 1)
		await inlineHost.vm.$nextTick()

		expect(inlineHost.text()).toContain('1 of 2 Selected')
		expect((searchInput.element as HTMLInputElement).value).toBe('Router')
		inlineHost.unmount()

		// Verify findLocalSubsetUsage & collectRemovedItemIds inspect item.host_id on node.data.items when pipelineItems is empty
		const store = seedStore()
		store.pipelineDetail!.host_ids = ['host-1', 'host-2']
		store.pipelineItems = []
		store.canvasGraph = null
		store.nodes = [
			{
				id: 'source-1',
				node_id: 'source-1',
				type: 'source',
				position: { x: 80, y: 160 },
				data: {
					id: 'source-1',
					hostCount: 2,
					itemCount: 2,
					item_count: 2,
					hosts: ['Router Core', 'Firewall Edge'],
					items: [
						{ item_id: 'item-1', host_id: 'host-1', name: 'CPU Load' },
						{ item_id: 'item-2', host_id: 'host-2', name: 'Packet Drop' }
					],
					health: { healthy: 2, warning: 0, error: 0 }
				}
			}
		]

		const ok = await store.applyPipelineHosts(['host-2'])
		expect(ok).toBe(false)
		expect(store.subsetConfirmation.isOpen).toBe(true)
		expect(store.subsetConfirmation.message).toContain('1 item(s)')

		// Verify 409 optimistic lock conflict refreshes both pipelineDetail and canvasGraph.last_updated
		store.canvasGraph = {
			pipeline_id: 'pipe-1',
			canvas_id: 'canvas-1',
			last_updated: '2026-11-01T00:00:00Z',
			graph: { nodes: [], relations: [] }
		} as any
		vi.spyOn(pipelinesApi, 'updatePipeline').mockRejectedValueOnce({
			status: 409,
			data: { error: { detail: 'Optimistic lock conflict' } }
		})
		vi.spyOn(pipelinesApi, 'getPipelineById').mockResolvedValueOnce({
			success: true,
			data: {
				...store.pipelineDetail,
				last_updated: '2026-11-01T09:00:00Z',
				canvases: [{ canvas_id: 'canvas-1', last_updated: '2026-11-01T09:00:00Z' }]
			}
		} as any)

		const forceConfirmed = await store.confirmSubsetForceUpdate()
		expect(forceConfirmed).toBe(false)
		expect(store.pipelineDetail?.last_updated).toBe('2026-11-01T09:00:00Z')
		expect(store.canvasGraph?.last_updated).toBe('2026-11-01T09:00:00Z')

		// Verify in-flight guard prevents duplicate getHosts / getSinks calls when modal open is triggered repeatedly
		let resolveHosts!: (val: any) => void
		const getHostsSpy = vi.spyOn(hostsApi, 'getHosts').mockImplementation(
			() =>
				new Promise((resolve) => {
					resolveHosts = resolve
				}) as any
		)
		const firstOpen = store.openHostSelectorModal()
		const secondOpen = store.openHostSelectorModal()
		expect(getHostsSpy).toHaveBeenCalledTimes(1)
		resolveHosts({ success: true, data: { hosts: [], total: 0 } })
		await Promise.all([firstOpen, secondOpen])
		expect(store.loadingResources.hosts).toBe(false)
	})

	it('supports lazy-100 actual total display, Select All with exclude list, and deferred fetch-all on Apply in HostSelectorPopover', async () => {
		const loadedHosts = Array.from({ length: 100 }, (_, idx) => ({
			id: `host-${idx}`,
			name: `Host ${idx}`,
			ip: `10.0.0.${idx % 255}`,
			environment: 'prod' as const
		}))
		const allIds = Array.from({ length: 250 }, (_, idx) => `host-${idx}`)
		const onResolveAllIds = vi.fn().mockResolvedValue(allIds)

		const wrapper = mount(HostSelectorPopover, {
			props: {
				inline: true,
				hosts: loadedHosts,
				totalCount: 250,
				selectedIds: [],
				onResolveAllIds
			},
			global: selectorMountGlobal
		})

		expect(wrapper.text()).toContain('0 of 250 Selected')

		// Click Select All -> enters selectAllMode without calling onResolveAllIds yet
		const selectAllBtn = wrapper.findAll('button').find((b) => b.text() === 'Select All')!
		await selectAllBtn.trigger('click')
		expect(onResolveAllIds).not.toHaveBeenCalled()
		expect(wrapper.text()).toContain('250 of 250 Selected')

		// Uncheck first 2 rendered hosts -> excludedIds tracks them
		const rows = wrapper.findAll('.cursor-pointer')
		expect(rows.length).toBeLessThan(50) // virtualized: does not mount all 100 rows
		await rows[0]!.trigger('click')
		await rows[1]!.trigger('click')
		expect(wrapper.text()).toContain('248 of 250 Selected')

		// Click Apply Selection -> resolves all IDs and filters out the 2 unchecked hosts
		const applyBtn = wrapper
			.findAll('button')
			.find((b) => b.text().includes('Apply Selection'))!
		await applyBtn.trigger('click')
		await vi.dynamicImportSettled()
		await Promise.resolve()

		expect(onResolveAllIds).toHaveBeenCalledWith('')
		const emitted = wrapper.emitted('apply')
		expect(emitted).toBeTruthy()
		const appliedIds = emitted![0]![0] as string[]
		expect(appliedIds).toHaveLength(248)
		expect(appliedIds).not.toContain('host-0')
		expect(appliedIds).not.toContain('host-1')
		expect(appliedIds).toContain('host-249')
	})

	it('disables Select All while server-side search debounce is pending and avoids duplicate reset fetch when modal reopens while loading', async () => {
		vi.useFakeTimers()
		try {
			const hosts = [
				{ id: 'host-1', name: 'Web 1', ip: '10.0.0.1', environment: 'prod' as const },
				{ id: 'host-2', name: 'DB 1', ip: '10.0.0.2', environment: 'prod' as const }
			]
			const onSearch = vi.fn().mockResolvedValue(undefined)

			const wrapper = mount(HostSelectorPopover, {
				props: {
					modelValue: true,
					inline: true,
					hosts,
					totalCount: 2,
					selectedIds: [],
					loading: false,
					onSearch
				},
				global: selectorMountGlobal
			})

			const input = wrapper.find('input')
			const selectAllBtn = () =>
				wrapper.findAll('button').find((b) => b.text() === 'Select All')!

			expect(selectAllBtn().attributes('disabled')).toBeUndefined()

			await input.setValue('Web')
			expect(selectAllBtn().attributes('disabled')).toBeDefined()

			await vi.advanceTimersByTimeAsync(350)
			expect(onSearch).toHaveBeenCalledTimes(1)
			expect(onSearch).toHaveBeenCalledWith('Web')
			expect(selectAllBtn().attributes('disabled')).toBeUndefined()

			// Close and reopen while parent store already started loading -> should not trigger duplicate onSearch('')
			await wrapper.setProps({ modelValue: false, inline: false })
			await wrapper.setProps({ modelValue: true, loading: true })
			expect(onSearch).toHaveBeenCalledTimes(1)
			wrapper.unmount()
		} finally {
			vi.useRealTimers()
		}
	})

	it('resets cached hosts and sinks when loadBuilder switches pipelines', async () => {
		const store = seedStore()
		expect(store.availableHosts.length).toBeGreaterThan(0)
		expect(store.availableSinks.length).toBeGreaterThan(0)

		vi.spyOn(pipelinesApi, 'getPipelineById').mockResolvedValueOnce({
			success: true,
			data: {
				pipeline_id: 'pipe-2',
				name: 'Pipeline 2',
				host_ids: [],
				sink_ids: [],
				canvases: []
			}
		} as any)
		vi.spyOn(hostsApi, 'getHosts').mockRejectedValueOnce(new Error('Cluster unreachable'))
		vi.spyOn(sinksApi, 'getSinks').mockRejectedValueOnce(new Error('Cluster unreachable'))

		await store.loadBuilder('pipe-2', 'default', 'http://cluster-b.test:8080')

		expect(store.availableHosts).toEqual([])
		expect(store.availableSinks).toEqual([])
		expect(store.hostById.size).toBe(0)
		expect(store.sinkById.size).toBe(0)
	})

	it('skips debounced onSearch when selector closes before debounce expires', async () => {
		vi.useFakeTimers()
		try {
			const hosts = [
				{ id: 'host-1', name: 'Web 1', ip: '10.0.0.1', environment: 'prod' as const }
			]
			const onSearch = vi.fn().mockResolvedValue(undefined)

			const wrapper = mount(HostSelectorPopover, {
				props: {
					modelValue: true,
					inline: true,
					hosts,
					totalCount: 1,
					selectedIds: [],
					onSearch
				},
				global: selectorMountGlobal
			})

			await wrapper.find('input').setValue('Web')
			await wrapper.setProps({ modelValue: false, inline: false })

			await vi.advanceTimersByTimeAsync(350)
			expect(onSearch).not.toHaveBeenCalled()
			wrapper.unmount()
		} finally {
			vi.useRealTimers()
		}
	})

	it('preserves unfiltered total in header badge during server-side search', async () => {
		vi.useFakeTimers()
		try {
			const allHosts = Array.from({ length: 10 }, (_, idx) => ({
				id: `host-${idx}`,
				name: `Host ${idx}`,
				ip: `10.0.0.${idx}`,
				environment: 'prod' as const
			}))
			const selectedIds = Array.from({ length: 150 }, (_, idx) => `host-${idx}`)

			const onSearch = vi.fn(async (q: string) => {
				if (q === 'Host 0') {
					await wrapper.setProps({
						hosts: [allHosts[0]!],
						totalCount: 5
					})
				}
			})

			const wrapper = mount(HostSelectorPopover, {
				props: {
					modelValue: true,
					inline: true,
					hosts: allHosts,
					totalCount: 250,
					selectedIds,
					onSearch
				},
				global: selectorMountGlobal
			})

			expect(wrapper.text()).toContain('150 of 250 Selected')

			await wrapper.find('input').setValue('Host 0')
			await vi.advanceTimersByTimeAsync(350)

			expect(onSearch).toHaveBeenCalledWith('Host 0')
			expect(wrapper.text()).toContain('150 of 250 Selected')
			wrapper.unmount()
		} finally {
			vi.useRealTimers()
		}
	})

	it('aborts applySelection commit if selector unmounts while onResolveAllIds is in flight', async () => {
		const loadedHosts = Array.from({ length: 10 }, (_, idx) => ({
			id: `host-${idx}`,
			name: `Host ${idx}`,
			ip: `10.0.0.${idx}`,
			environment: 'prod' as const
		}))
		let resolveAll!: (ids: string[]) => void
		const onResolveAllIds = vi.fn(
			() =>
				new Promise<string[]>((resolve) => {
					resolveAll = resolve
				})
		)

		const wrapper = mount(HostSelectorPopover, {
			props: {
				inline: true,
				hosts: loadedHosts,
				totalCount: 250,
				selectedIds: [],
				onResolveAllIds
			},
			global: selectorMountGlobal
		})

		const selectAllBtn = wrapper.findAll('button').find((b) => b.text() === 'Select All')!
		await selectAllBtn.trigger('click')

		const applyBtn = wrapper
			.findAll('button')
			.find((b) => b.text().includes('Apply Selection'))!
		await applyBtn.trigger('click')

		wrapper.unmount()
		resolveAll(['host-0', 'host-1'])
		await Promise.resolve()
		await Promise.resolve()

		expect(wrapper.emitted('apply')).toBeUndefined()
	})

	it('keeps BuilderSinkNode name via sinkById and EdgeItemsModal host options via pipelineDetail when selector lists are filtered', async () => {
		const store = seedStore()
		store.pipelineDetail!.host_ids = ['host-1', 'host-2', 'host-999']
		store.pipelineItems = [
			{ item_id: 'item-1', host_id: 'host-1', host_name: 'Router Core', name: 'CPU Load' },
			{ item_id: 'item-999', host_id: 'host-999', host_name: 'Remote Switch', name: 'Uptime' }
		]

		vi.spyOn(sinksApi, 'getSinks')
			.mockResolvedValueOnce({
				success: true,
				data: {
					sinks: [{ sink_id: 'sink-1', name: 'Kafka Primary', type: 'kafka' }],
					total: 1
				}
			} as any)
			.mockResolvedValueOnce({
				success: true,
				data: { sinks: [], total: 0 }
			} as any)

		vi.spyOn(hostsApi, 'getHosts')
			.mockResolvedValueOnce({
				success: true,
				data: {
					hosts: [{ host_id: 'host-1', name: 'Router Core', ip: '10.0.0.1' }],
					total: 1
				}
			} as any)
			.mockResolvedValueOnce({
				success: true,
				data: { hosts: [], total: 0 }
			} as any)

		await store.searchSelectorSinks('')
		await store.searchSelectorSinks('no-match')
		expect(store.availableSinks).toEqual([])

		const sinkNodeWrapper = mount(BuilderSinkNode, {
			props: {
				id: 'sink-node-1',
				data: { sinkId: 'sink-1' } as any
			},
			global: {
				stubs: { Handle: true, UiIcon: true }
			}
		})
		expect(sinkNodeWrapper.text()).toContain('Kafka Primary')
		sinkNodeWrapper.unmount()

		await store.searchSelectorHosts('')
		await store.searchSelectorHosts('no-match')
		expect(store.availableHosts).toEqual([])

		store.edgeItemModal = { isOpen: true, edgeId: 'edge-1' }
		store.edges = [{ id: 'edge-1', data: { selectedItemIds: [] } }] as any

		const edgeModalWrapper = mount(EdgeItemsModal, {
			global: {
				stubs: {
					AppInputText: true,
					AppInputSelect: {
						props: ['options'],
						template:
							'<select data-testid="host-filter"><option v-for="o in options" :key="o.value" :value="o.value">{{ o.label }}</option></select>'
					},
					UiIcon: true
				}
			}
		})

		const optionTexts = edgeModalWrapper
			.findAll('[data-testid="host-filter"] option')
			.map((o) => o.text())
		expect(optionTexts).toContain('Router Core')
		expect(optionTexts).toContain('Remote Switch')
		expect(optionTexts).toContain('host-2')
		edgeModalWrapper.unmount()
	})

	it('throws on resolveAllSelectorHostIds and resolveAllSelectorSinkIds when API returns undefined or success false', async () => {
		const store = seedStore()

		vi.spyOn(hostsApi, 'getHosts')
			.mockResolvedValueOnce(undefined)
			.mockResolvedValueOnce({ success: false, message: 'Host query failed' } as any)
		vi.spyOn(sinksApi, 'getSinks')
			.mockResolvedValueOnce(undefined)
			.mockResolvedValueOnce({ success: false, message: 'Sink query failed' } as any)

		await expect(store.resolveAllSelectorHostIds()).rejects.toThrow(
			'Failed to load available hosts.'
		)
		await expect(store.resolveAllSelectorHostIds()).rejects.toThrow('Host query failed')
		await expect(store.resolveAllSelectorSinkIds()).rejects.toThrow(
			'Failed to load available sinks.'
		)
		await expect(store.resolveAllSelectorSinkIds()).rejects.toThrow('Sink query failed')
	})

	it('supersedes an in-flight filtered search when reopening host selector modal', async () => {
		const store = seedStore()
		let resolveFiltered!: (val: any) => void
		const filteredPromise = new Promise((resolve) => {
			resolveFiltered = resolve
		})

		const getHostsSpy = vi
			.spyOn(hostsApi, 'getHosts')
			.mockReturnValueOnce(filteredPromise as any)
			.mockResolvedValueOnce({
				success: true,
				data: {
					hosts: [{ host_id: 'host-all-1', name: 'All Host 1', ip: '10.0.0.1' }],
					total: 1
				}
			} as any)

		const filteredSearch = store.searchSelectorHosts('xyz')
		expect(store.loadingResources.hosts).toBe(true)
		expect(store.hostsSearchQuery).toBe('xyz')

		store.closeHostSelectorModal()
		await store.openHostSelectorModal()

		resolveFiltered({
			success: true,
			data: {
				hosts: [{ host_id: 'host-xyz', name: 'Filtered XYZ', ip: '10.0.0.99' }],
				total: 1
			}
		})
		await filteredSearch

		expect(getHostsSpy).toHaveBeenNthCalledWith(1, { limit: 100, name: 'xyz' }, undefined)
		expect(getHostsSpy).toHaveBeenNthCalledWith(2, { limit: 100 }, undefined)
		expect(store.hostsSearchQuery).toBe('')
		expect(store.availableHosts).toHaveLength(1)
		expect(store.availableHosts[0].host_id).toBe('host-all-1')
	})

	it('deselects all on first Select All click when initial selectedIds already cover totalCount with unloaded items', async () => {
		const loadedHosts = Array.from({ length: 100 }, (_, idx) => ({
			id: `host-${idx}`,
			name: `Host ${idx}`,
			ip: `10.0.0.${idx % 255}`,
			environment: 'prod' as const
		}))
		const initialSelected = Array.from({ length: 250 }, (_, idx) => `host-${idx}`)
		const onResolveAllIds = vi.fn().mockResolvedValue(initialSelected)

		const wrapper = mount(HostSelectorPopover, {
			props: {
				inline: true,
				hosts: loadedHosts,
				totalCount: 250,
				selectedIds: initialSelected,
				onResolveAllIds
			},
			global: selectorMountGlobal
		})

		expect(wrapper.text()).toContain('250 of 250 Selected')

		const selectAllBtn = wrapper.findAll('button').find((b) => b.text() === 'Select All')!
		await selectAllBtn.trigger('click')
		expect(wrapper.text()).toContain('0 of 250 Selected')
		expect(onResolveAllIds).not.toHaveBeenCalled()
		wrapper.unmount()
	})

	it('deselects cleanly without calling resolveAllIds when all pages are loaded after Select All', async () => {
		const page1 = Array.from({ length: 10 }, (_, idx) => ({
			id: `sink-${idx}`,
			name: `Sink ${idx}`,
			type: 'kafka'
		}))
		const allSinks = Array.from({ length: 15 }, (_, idx) => ({
			id: `sink-${idx}`,
			name: `Sink ${idx}`,
			type: 'kafka'
		}))
		const onResolveAllIds = vi.fn().mockResolvedValue(allSinks.map((s) => s.id))

		const wrapper = mount(SinkSelectorPopover, {
			props: {
				inline: true,
				sinks: page1,
				totalCount: 15,
				selectedIds: [],
				onResolveAllIds
			},
			global: selectorMountGlobal
		})

		const selectAllBtn = wrapper.findAll('button').find((b) => b.text() === 'Select All')!
		await selectAllBtn.trigger('click')
		expect(wrapper.text()).toContain('15 of 15 Selected')

		// Simulate loading all remaining items so hasUnloadedItems becomes false
		await wrapper.setProps({ sinks: allSinks })

		// Click Select All again -> should exit selectAllMode and clear selection
		await selectAllBtn.trigger('click')
		expect(wrapper.text()).toContain('0 of 15 Selected')

		const applyBtn = wrapper
			.findAll('button')
			.find((b) => b.text().includes('Apply Selection'))!
		await applyBtn.trigger('click')
		await Promise.resolve()

		expect(onResolveAllIds).not.toHaveBeenCalled()
		expect(wrapper.emitted('apply')?.[0]?.[0]).toEqual([])
		wrapper.unmount()
	})

	it('enters selectAllMode instead of clearing when selectedIds has stale IDs matching totalCount but loaded items are unchecked', async () => {
		const loadedHosts = Array.from({ length: 10 }, (_, idx) => ({
			id: `host-${idx}`,
			name: `Host ${idx}`,
			ip: `10.0.0.${idx}`,
			environment: 'prod' as const
		}))
		// 20 stale/deleted IDs (none of the currently loaded host-0..9 are selected)
		const staleSelected = Array.from({ length: 20 }, (_, idx) => `deleted-host-${idx}`)
		const onResolveAllIds = vi.fn().mockResolvedValue([])

		const wrapper = mount(HostSelectorPopover, {
			props: {
				inline: true,
				hosts: loadedHosts,
				totalCount: 20,
				selectedIds: staleSelected,
				onResolveAllIds
			},
			global: selectorMountGlobal
		})

		const selectAllBtn = wrapper.findAll('button').find((b) => b.text() === 'Select All')!
		await selectAllBtn.trigger('click')
		expect(wrapper.text()).toContain('20 of 20 Selected')
		// First rendered row should now be checked (selectAllMode active)
		const firstRow = wrapper.find('.cursor-pointer')
		expect(firstRow.classes()).toContain('bg-sky-50/80')
		wrapper.unmount()
	})

	it('resets unfilteredTotal when dialog reopens while loading so immediate search does not show stale total', async () => {
		vi.useFakeTimers()
		try {
			const hosts = [
				{ id: 'host-1', name: 'Alpha 1', ip: '10.0.0.1', environment: 'prod' as const }
			]
			const onSearch = vi.fn(async () => {
				await wrapper.setProps({ hosts, totalCount: 3, loading: false })
			})

			const wrapper = mount(HostSelectorPopover, {
				props: {
					modelValue: true,
					inline: false,
					hosts,
					totalCount: 500,
					selectedIds: [],
					loading: false,
					onSearch
				},
				global: selectorMountGlobal
			})

			expect(wrapper.text()).toContain('0 of 500 Selected')

			// Close and reopen while loading is true (e.g., cluster now has fewer hosts)
			await wrapper.setProps({ modelValue: false })
			await wrapper.setProps({ modelValue: true, loading: true, hosts: [], totalCount: 0 })

			await wrapper.find('input').setValue('Alpha')
			await vi.advanceTimersByTimeAsync(350)

			expect(onSearch).toHaveBeenCalledWith('Alpha')
			expect(wrapper.text()).toContain('0 of 3 Selected')
			expect(wrapper.text()).not.toContain('0 of 500 Selected')
			wrapper.unmount()
		} finally {
			vi.useRealTimers()
		}
	})

	it('awaits sink hydration and hydrates hosts non-blocking in background during loadBuilder', async () => {
		const store = usePipelineBuildStore()
		let resolveHostHydrate!: (val: any) => void
		const hostHydratePromise = new Promise((resolve) => {
			resolveHostHydrate = resolve
		})

		vi.spyOn(pipelinesApi, 'getPipelineById').mockResolvedValueOnce({
			success: true,
			data: {
				pipeline_id: 'pipe-hydrate',
				name: 'Hydrate Pipeline',
				host_ids: ['host-1', 'host-250'],
				sink_ids: ['sink-1', 'sink-180'],
				canvases: [{ canvas_id: 'canvas-1', name: 'Main' }]
			}
		} as any)
		vi.spyOn(pipelinesApi, 'getPipelineGraph').mockResolvedValueOnce({
			success: true,
			data: {
				pipeline_id: 'pipe-hydrate',
				canvas_id: 'canvas-1',
				graph: {
					nodes: [
						{ node_id: 'source-1', kind: 'source', position: { x: 0, y: 0 }, data: {} }
					],
					edges: []
				}
			}
		} as any)
		vi.spyOn(pipelinesApi, 'getPipelineNode').mockResolvedValueOnce({
			success: true,
			data: { node_id: 'source-1', kind: 'source', data: { items: [], total: 0 } }
		} as any)
		vi.spyOn(itemsApi, 'getItems').mockResolvedValueOnce({
			success: true,
			data: { items: [], total: 0 }
		} as any)

		const getHostsSpy = vi
			.spyOn(hostsApi, 'getHosts')
			.mockResolvedValueOnce({
				success: true,
				data: {
					hosts: [{ host_id: 'host-1', name: 'Host 1', ip: '10.0.0.1' }],
					total: 300
				}
			} as any)
			.mockReturnValueOnce(hostHydratePromise as any)

		const getSinksSpy = vi
			.spyOn(sinksApi, 'getSinks')
			.mockResolvedValueOnce({
				success: true,
				data: {
					sinks: [{ sink_id: 'sink-1', name: 'Sink 1', type: 'kafka' }],
					total: 200
				}
			} as any)
			.mockResolvedValueOnce({
				success: true,
				data: {
					sinks: [{ sink_id: 'sink-180', name: 'Sink 180', type: 'clickhouse' }],
					total: 1
				}
			} as any)

		await store.loadBuilder('pipe-hydrate', 'canvas-1', 'http://cluster-a.test:8080')

		// Canvas finishes loading while background host hydration is still pending
		expect(store.loading.canvas).toBe(false)
		expect(getSinksSpy).toHaveBeenNthCalledWith(2, { sink_ids: ['sink-180'], limit: 1 }, [
			'http://cluster-a.test:8080'
		])
		expect(store.sinkById.get('sink-180')?.name).toBe('Sink 180')
		expect(store.hostById.get('host-250')).toBeUndefined()
		const sourceNodeBefore = store.nodes.find((n: any) => n.type === 'source') as any
		expect(sourceNodeBefore?.data?.hosts).toEqual(['Host 1', 'host-250'])

		resolveHostHydrate({
			success: true,
			data: {
				hosts: [{ host_id: 'host-250', name: 'Host 250', ip: '10.0.1.250' }],
				total: 1
			}
		})
		await vi.waitFor(() => {
			const sourceNodeAfter = store.nodes.find((n: any) => n.type === 'source') as any
			expect(sourceNodeAfter?.data?.hosts).toEqual(['Host 1', 'Host 250'])
		})

		expect(getHostsSpy).toHaveBeenNthCalledWith(2, { host_ids: ['host-250'], limit: 1 }, [
			'http://cluster-a.test:8080'
		])
		expect(store.hostById.get('host-250')?.name).toBe('Host 250')
	})

	it('caps background host hydration to 200 IDs', async () => {
		const store = usePipelineBuildStore()
		const manyHostIds = Array.from({ length: 260 }, (_, idx) => `host-${idx}`)

		vi.spyOn(pipelinesApi, 'getPipelineById').mockResolvedValueOnce({
			success: true,
			data: {
				pipeline_id: 'pipe-many',
				name: 'Many Hosts Pipeline',
				host_ids: manyHostIds,
				sink_ids: [],
				canvases: []
			}
		} as any)
		vi.spyOn(itemsApi, 'getItems').mockResolvedValueOnce({
			success: true,
			data: { items: [], total: 0 }
		} as any)
		vi.spyOn(sinksApi, 'getSinks').mockResolvedValueOnce({
			success: true,
			data: { sinks: [], total: 0 }
		} as any)

		const hydratedIds: string[] = []
		vi.spyOn(hostsApi, 'getHosts').mockImplementation(async (query: any) => {
			if (query?.host_ids) {
				hydratedIds.push(...query.host_ids)
				return {
					success: true,
					data: {
						hosts: query.host_ids.map((id: string) => ({
							host_id: id,
							name: `Name ${id}`,
							ip: '10.0.0.1'
						})),
						total: query.host_ids.length
					}
				} as any
			}
			return {
				success: true,
				data: { hosts: [], total: 260 }
			} as any
		})

		await store.loadBuilder('pipe-many', 'default', 'http://cluster-a.test:8080')
		await vi.waitFor(() => {
			expect(hydratedIds).toHaveLength(200)
		})
		expect(store.hostById.get('host-199')?.name).toBe('Name host-199')
		expect(store.hostById.get('host-200')).toBeUndefined()
	})

	it('syncs selectAllBaseTotal when unfiltered totalCount updates while selectAllMode is active', async () => {
		vi.useFakeTimers()
		try {
			const hosts = Array.from({ length: 10 }, (_, idx) => ({
				id: `host-${idx}`,
				name: `Host ${idx}`,
				ip: `10.0.0.${idx}`,
				environment: 'prod' as const
			}))
			const onSearch = vi.fn(async (q: string) => {
				if (q === 'Host 0') {
					await wrapper.setProps({ hosts: [hosts[0]!], totalCount: 1 })
				} else {
					await wrapper.setProps({ hosts, totalCount: 240 })
				}
			})
			const onResolveAllIds = vi.fn().mockResolvedValue([])

			const wrapper = mount(HostSelectorPopover, {
				props: {
					modelValue: true,
					inline: true,
					hosts,
					totalCount: 250,
					selectedIds: [],
					onSearch,
					onResolveAllIds
				},
				global: selectorMountGlobal
			})

			const selectAllBtn = wrapper.findAll('button').find((b) => b.text() === 'Select All')!
			await selectAllBtn.trigger('click')
			expect(wrapper.text()).toContain('250 of 250 Selected')

			// Uncheck one item
			await wrapper.find('.cursor-pointer').trigger('click')
			expect(wrapper.text()).toContain('249 of 250 Selected')

			// Search and then clear search -> server now reports 240 total
			await wrapper.find('input').setValue('Host 0')
			await vi.advanceTimersByTimeAsync(350)
			await wrapper.find('input').setValue('')
			await vi.advanceTimersByTimeAsync(350)

			expect(wrapper.text()).toContain('239 of 240 Selected')
			wrapper.unmount()
		} finally {
			vi.useRealTimers()
		}
	})
})

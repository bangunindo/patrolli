// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import { usePipelineBuildStore } from '@/stores/pipeline.builder.store'
import pipelineManagementApi from '@/app/api/pipeline.management.api'
import hostsApi from '@/app/api/hosts.api'
import sinksApi from '@/app/api/sinks.api'

describe('Pipeline Builder Store & Canvas Integration', () => {
	beforeEach(() => {
		setActivePinia(createPinia())
		vi.clearAllMocks()

		vi.spyOn(hostsApi, 'getHosts').mockResolvedValue({
			success: true,
			data: {
				hosts: [
					{ host_id: 'host-1', name: 'Production Host 01' },
					{ host_id: 'host-2', name: 'Staging Host 02' }
				]
			}
		} as any)

		vi.spyOn(sinksApi, 'getSinks').mockResolvedValue({
			success: true,
			data: {
				sinks: [{ sink_id: 'sink-1', name: 'Kafka Sink', type: 'kafka' }]
			}
		} as any)

		vi.spyOn(pipelineManagementApi, 'getPipelineNode').mockResolvedValue({
			success: true,
			data: {
				last_updated: '2026-10-21T00:00:00Z',
				content: { kind: 'source', data: { items: [], total: 0 } }
			}
		} as any)

		vi.spyOn(pipelineManagementApi, 'updatePipelineNode').mockResolvedValue({
			success: true,
			data: {}
		} as any)
	})

	const mockSavedGraph = {
		canvas_id: 'canvas-02',
		pipeline_management_id: 'pipe-02',
		graph: {
			nodes: [
				{
					kind: 'source',
					node_id: 'source-node-id',
					position_x: 80,
					position_y: 160,
					data: { items: ['item-1'] }
				},
				{
					kind: 'sink',
					node_id: 'sink-node-id',
					position_x: 720,
					position_y: 220,
					data: { sink_id: 'sink-1' }
				}
			],
			relations: [
				{
					from_node_id: 'source-node-id',
					to_node_id: 'sink-node-id',
					label: 'Custom Stream Filter'
				}
			]
		},
		last_updated: '2026-10-21T00:00:00Z'
	}

	it('sets canvas to empty state when response api graph.nodes is empty', async () => {
		vi.spyOn(pipelineManagementApi, 'getPipelineById').mockResolvedValue({
			success: true,
			data: {
				pipeline_management_id: 'pipe-01',
				name: 'Telemetry Pipeline',
				adm_status: 1,
				visibility: 1,
				host_ids: ['host-1', 'host-2'],
				sink_ids: [],
				canvases: [{ canvas_id: 'canvas-01', last_updated: '2026-10-21T00:00:00Z' }]
			}
		} as any)

		vi.spyOn(pipelineManagementApi, 'getPipelineGraph').mockResolvedValue({
			success: true,
			data: {
				canvas_id: 'canvas-01',
				pipeline_management_id: 'pipe-01',
				graph: { nodes: [], relations: [] },
				last_updated: '2026-10-21T00:00:00Z'
			}
		} as any)

		const store = usePipelineBuildStore()
		await store.loadBuilder('pipe-01', 'canvas-01', 'http://cluster-1:8080')

		expect(store.pipelineId).toBe('pipe-01')
		expect(store.canvasId).toBe('canvas-01')
		expect(store.nodes.length).toBe(0)
		expect(store.edges.length).toBe(0)
		expect(store.isCanvasEmpty).toBe(true)

		// Click create / addSourceNode switches from empty state to active canvas
		store.addSourceNode()
		expect(store.nodes.length).toBe(1)
		expect(store.nodes[0].type).toBe('source')
		expect(store.isCanvasEmpty).toBe(false)
	})

	it('loads saved graph nodes and relations when response api graph.nodes is populated', async () => {
		vi.spyOn(pipelineManagementApi, 'getPipelineById').mockResolvedValue({
			success: true,
			data: {
				pipeline_management_id: 'pipe-02',
				name: 'Full Pipeline',
				adm_status: 1,
				visibility: 1,
				host_ids: ['host-1'],
				sink_ids: ['sink-1'],
				canvases: [{ canvas_id: 'canvas-02', last_updated: '2026-10-21T00:00:00Z' }]
			}
		} as any)

		vi.spyOn(pipelineManagementApi, 'getPipelineGraph').mockResolvedValue({
			success: true,
			data: mockSavedGraph
		} as any)

		const store = usePipelineBuildStore()
		await store.loadBuilder('pipe-02', 'canvas-02')

		expect(store.nodes.length).toBe(2)
		expect(store.isCanvasEmpty).toBe(false)
		expect(store.nodes[0].type).toBe('source')
		expect(store.nodes[0].node_id).toBe('source-node-id')
		expect(store.nodes[1].type).toBe('sink')
		expect(store.nodes[1].node_id).toBe('sink-node-id')
		expect(store.nodes[1].data.name).toBe('Sink Target')
		expect(store.nodes[1].data.sinkName).toBe('Kafka Sink')
		expect(store.edges.length).toBe(1)
		expect(store.edges[0].target).toBe(store.nodes[1].id)
		expect(store.edges[0].data.label).toBe('Custom Stream Filter')
	})

	it('supports drag-and-drop node addition and saves graph to API', async () => {
		vi.spyOn(pipelineManagementApi, 'getPipelineById').mockResolvedValue({
			success: true,
			data: {
				pipeline_management_id: 'pipe-02',
				name: 'Full Pipeline',
				adm_status: 1,
				visibility: 1,
				host_ids: ['host-1'],
				sink_ids: ['sink-1'],
				canvases: [{ canvas_id: 'canvas-02', last_updated: '2026-10-21T00:00:00Z' }]
			}
		} as any)

		vi.spyOn(pipelineManagementApi, 'getPipelineGraph').mockResolvedValue({
			success: true,
			data: mockSavedGraph
		} as any)

		const updateSpy = vi.spyOn(pipelineManagementApi, 'updatePipelineGraph').mockResolvedValue({
			success: true,
			data: {
				canvas_id: 'canvas-02',
				pipeline_management_id: 'pipe-02',
				graph: { nodes: [], relations: [] },
				last_updated: '2026-10-21T00:01:00Z'
			}
		} as any)

		const store = usePipelineBuildStore()
		await store.loadBuilder('pipe-02', 'canvas-02', 'http://c1.test:8080')

		// Add processor node
		const proc = store.addProcessorNode(400, 200, 'Custom Transform')
		expect(store.isDirty).toBe(true)
		expect(store.nodes.length).toBe(3)
		expect(proc.node_id).toMatch(
			/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i
		)

		// Connect source to processor
		store.connectNodes(store.nodes[0].id, proc.id)
		expect(store.edges.length).toBe(2)

		// Save graph
		const saved = await store.saveGraph()
		expect(saved).toBe(true)
		expect(updateSpy).toHaveBeenCalledWith(
			{
				pipeline_management_id: 'pipe-02',
				canvas_id: 'canvas-02'
			},
			expect.objectContaining({
				graph: expect.any(Object),
				last_updated: expect.any(String)
			}),
			['http://c1.test:8080']
		)
		expect(store.isDirty).toBe(false)
	})

	it('splits edge and inserts processor node without overlapping', async () => {
		const store = usePipelineBuildStore()
		await store.loadBuilder('pipe-02', 'canvas-02')

		// Initially 2 nodes (source, sink) and 1 edge
		expect(store.nodes.length).toBe(2)
		expect(store.edges.length).toBe(1)
		const sourceNode = store.nodes[0]
		const sinkNode = store.nodes[1]
		const targetEdgeId = store.edges[0].id

		// User clicks plus button on edge to insert processor
		const insertedProc = store.insertProcessorBetween(targetEdgeId, 450, 200)
		expect(insertedProc).toBeDefined()
		expect(insertedProc?.node_id).toMatch(
			/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i
		)

		// Old edge must be removed, 2 new connecting edges created
		expect(store.nodes.length).toBe(3)
		expect(store.edges.length).toBe(2)
		expect(store.edges.some((e) => e.id === targetEdgeId)).toBe(false)
		expect(
			store.edges.some((e) => e.source === sourceNode.id && e.target === insertedProc!.id)
		).toBe(true)
		expect(
			store.edges.some((e) => e.source === insertedProc!.id && e.target === sinkNode.id)
		).toBe(true)
	})

	it('supports deleting source node and its connected edges', async () => {
		const store = usePipelineBuildStore()
		await store.loadBuilder('pipe-02', 'canvas-02')

		expect(store.nodes.length).toBe(2)
		expect(store.edges.length).toBe(1)
		const sourceNode = store.nodes.find((n: any) => n.type === 'source')!
		expect(sourceNode).toBeDefined()

		// Delete source node
		store.removeNode(sourceNode.id)
		expect(store.nodes.some((n: any) => n.type === 'source')).toBe(false)
		expect(store.nodes.length).toBe(1)
		// Connected edges should also be removed
		expect(store.edges.length).toBe(0)
		expect(store.isDirty).toBe(true)
	})

	it('opens edge item selector when connecting source to processor and applies items', () => {
		const store = usePipelineBuildStore()
		const sourceNode = store.addSourceNode(100, 100)
		const procNode = store.addProcessorNode(300, 100, 'Data Processor')

		expect(store.edgeItemModal.isOpen).toBe(false)

		// Connect Source -> Processor
		store.connectNodes(sourceNode.id, procNode.id)

		// Edge item modal must automatically open
		expect(store.edgeItemModal.isOpen).toBe(true)
		const edgeId = store.edgeItemModal.edgeId!
		expect(edgeId).toBe(`edge-${sourceNode.id}-${procNode.id}`)

		// Apply selected items
		store.applyEdgeItems(edgeId, ['item-101', 'item-102', 'item-103'])
		expect(store.edgeItemModal.isOpen).toBe(false)

		const edge = store.edges.find((e) => e.id === edgeId)
		expect(edge?.data?.selectedItemIds).toEqual(['item-101', 'item-102', 'item-103'])
		expect(edge?.data?.label).toBe('3 Items')

		// Target processor item count is updated
		const updatedProc = store.nodes.find((n) => n.id === procNode.id)
		expect(updatedProc?.data?.itemCount).toBe(3)
	})

	it('updates sink node metadata when selecting a registered sink', () => {
		const store = usePipelineBuildStore()
		store.availableSinks = [
			{
				sink_id: 'sink-kafka-1',
				name: 'Production Kafka',
				type: 'kafka',
				config: { brokers: ['kafka:9092'], topic: 'zarus.telemetry' }
			}
		]

		const sinkNode = store.addSinkNode(600, 200)
		expect(sinkNode.type).toBe('sink')
		expect(sinkNode.data.sinkId).toBeUndefined()
		expect(sinkNode.data.type).toBe('No sink selected')

		store.selectSinkForNode(sinkNode.id, 'sink-kafka-1')
		const updatedSink = store.nodes.find((n) => n.id === sinkNode.id)
		expect(updatedSink?.label).toBe('Sink Target')
		expect(updatedSink?.data?.name).toBe('Sink Target')
		expect(updatedSink?.data?.sinkName).toBe('Production Kafka')
		expect(updatedSink?.data?.sinkId).toBe('sink-kafka-1')
		expect(store.isDirty).toBe(true)
	})

	it('displays item_count on the edge from source to processor group from api graph response', async () => {
		const apiGraphResponse = {
			pipeline_management_id: '01a15d16-7882-7d92-a821-29e595d65c6b',
			canvas_id: '01a15d16-7886-7da2-8dae-842d241f3a85',
			last_updated: '2026-10-21T10:53:31.979464Z',
			graph: {
				nodes: [
					{
						node_id: '4c3bcb2e-058b-4c8f-8703-c9fbd62d9d44',
						position_x: 638,
						position_y: 283,
						kind: 'processor_group',
						data: {
							processors: []
						}
					},
					{
						node_id: '5d832e34-4201-4683-9c5d-0f7f886ccefd',
						position_x: 120,
						position_y: 220,
						kind: 'source',
						data: {
							item_count: 0
						}
					}
				],
				relations: [
					{
						from_node_id: '5d832e34-4201-4683-9c5d-0f7f886ccefd',
						to_node_id: '4c3bcb2e-058b-4c8f-8703-c9fbd62d9d44'
					}
				]
			},
			cluster_url: '/patrolli',
			cluster_name: 'patrolli'
		}

		vi.spyOn(pipelineManagementApi, 'getPipelineById').mockResolvedValue({
			success: true,
			data: {
				pipeline_management_id: '01a15d16-7882-7d92-a821-29e595d65c6b',
				name: 'Patrolli Pipeline',
				adm_status: 1,
				visibility: 1,
				host_ids: ['host-1'],
				sink_ids: [],
				canvases: [
					{
						canvas_id: '01a15d16-7886-7da2-8dae-842d241f3a85',
						last_updated: '2026-10-21T10:53:31.979464Z'
					}
				]
			}
		} as any)

		vi.spyOn(pipelineManagementApi, 'getPipelineGraph').mockResolvedValue({
			success: true,
			data: apiGraphResponse
		} as any)

		const store = usePipelineBuildStore()
		await store.loadBuilder(
			'01a15d16-7882-7d92-a821-29e595d65c6b',
			'01a15d16-7886-7da2-8dae-842d241f3a85'
		)

		expect(store.nodes.length).toBe(2)
		const sourceNode = store.nodes.find((n) => n.type === 'source')
		expect(sourceNode).toBeDefined()
		expect(sourceNode?.data?.itemCount).toBe(0)
		expect(sourceNode?.data?.item_count).toBe(0)

		expect(store.edges.length).toBe(1)
		const edge = store.edges[0]
		expect(edge.source).toBe('5d832e34-4201-4683-9c5d-0f7f886ccefd')
		expect(edge.target).toBe('4c3bcb2e-058b-4c8f-8703-c9fbd62d9d44')
		expect(edge.data?.label).toBe('0 Items')
	})

	it('generates UUID v7 format for newly added source, processor, and sink nodes', () => {
		const uuidV7Regex = /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i
		const store = usePipelineBuildStore()

		const source = store.addSourceNode()
		expect(source.node_id).toMatch(uuidV7Regex)
		expect(source.id).toBe(source.node_id)
		expect(source.data.node_id).toBe(source.node_id)

		const processor = store.addProcessorNode()
		expect(processor.node_id).toMatch(uuidV7Regex)
		expect(processor.id).toBe(processor.node_id)
		expect(processor.data.node_id).toBe(processor.node_id)

		const sink = store.addSinkNode()
		expect(sink.node_id).toMatch(uuidV7Regex)
		expect(sink.id).toBe(sink.node_id)
		expect(sink.data.node_id).toBe(sink.node_id)
	})
})

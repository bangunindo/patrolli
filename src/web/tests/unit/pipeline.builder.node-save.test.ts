// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import '@@/shared/types/array.type'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { usePipelineBuildStore } from '@/stores/pipeline.builder.store'
import { useProcessorStore } from '@/stores/processor.store'
import { useAppStore } from '@/stores/app.store'
import pipelinesApi from '@/app/api/pipelines.api'
import processorsApi from '@/app/api/processors.api'

describe('Pipeline Builder: Node Content Save Integration (updatePipelineNode)', () => {
	let store: ReturnType<typeof usePipelineBuildStore>

	beforeEach(() => {
		setActivePinia(createPinia())
		store = usePipelineBuildStore()
		store.pipelineId = 'pipe-1111-2222'
		store.canvasId = 'canvas-3333-4444'
		store.clusterUrl = 'http://cluster-test.local:8080'
		vi.clearAllMocks()
		vi.spyOn(processorsApi, 'get').mockResolvedValue({
			success: true,
			data: { processors: [], total: 0 }
		} as any)
	})

	it('successfully executes Stage 1 (updatePipelineGraph) followed by Stage 2 (updatePipelineNode) for all node types', async () => {
		// Mock nodes on canvas: 1 source, 1 processor_group, 1 sink
		store.nodes = [
			{
				id: 'source-node-1',
				type: 'source',
				label: 'Source',
				position: { x: 100, y: 150 },
				data: {
					node_id: 'source-node-1',
					items: ['item-uuid-1', 'item-uuid-2']
				}
			},
			{
				id: 'proc-group-node-2',
				type: 'processor',
				label: 'My Filters',
				position: { x: 350, y: 150 },
				data: {
					node_id: 'proc-group-node-2',
					name: 'My Filters',
					processors: [
						{
							processor_id: 'proc-uuid-1',
							name: 'Drop Passwords',
							module: 'drop_attributes',
							status: true,
							config: {
								type: 'drop_attributes',
								config: {
									keys: {
										record: ['password']
									}
								}
							}
						},
						{
							processor_id: 'proc-uuid-2',
							name: 'Inactive Filter',
							module: 'null',
							status: false
						}
					]
				}
			},
			{
				id: 'sink-node-3',
				type: 'sink',
				label: 'Kafka Sink',
				position: { x: 600, y: 150 },
				data: {
					node_id: 'sink-node-3',
					sinkId: 'sink-uuid-99'
				}
			}
		] as any

		store.edges = [
			{
				id: 'e1',
				source: 'source-node-1',
				target: 'proc-group-node-2'
			},
			{
				id: 'e2',
				source: 'proc-group-node-2',
				target: 'sink-node-3'
			}
		] as any

		// Stage 1 mock: updatePipelineGraph
		const graphSpy = vi.spyOn(pipelinesApi, 'updatePipelineGraph').mockResolvedValue({
			success: true,
			data: {
				pipeline_id: 'pipe-1111-2222',
				canvas_id: 'canvas-3333-4444',
				last_updated: '2026-10-22T10:00:00Z',
				graph: { nodes: [], relations: [] }
			}
		} as any)

		// Stage 2 mock: getPipelineNode for fresh last_updated and backend state
		const getNodeSpy = vi
			.spyOn(pipelinesApi, 'getPipelineNode')
			.mockImplementation((params) => {
				if (params.node_id === 'source-node-1') {
					return Promise.resolve({
						success: true,
						data: {
							node_id: 'source-node-1',
							last_updated: '2026-10-22T10:01:00Z',
							content: {
								kind: 'source',
								data: {
									// existing item in backend is item-uuid-1 and item-uuid-old
									items: [
										{ item_id: 'item-uuid-1' },
										{ item_id: 'item-uuid-old' }
									],
									total: 2
								}
							}
						}
					} as any)
				}
				if (params.node_id === 'proc-group-node-2') {
					return Promise.resolve({
						success: true,
						data: {
							node_id: 'proc-group-node-2',
							last_updated: '2026-10-22T10:02:00Z',
							content: {
								kind: 'processor_group',
								data: { processors: [] }
							}
						}
					} as any)
				}
				// sink-node-3
				return Promise.resolve({
					success: true,
					data: {
						node_id: 'sink-node-3',
						last_updated: '2026-10-22T10:03:00Z',
						content: {
							kind: 'sink',
							data: null
						}
					}
				} as any)
			})

		// Stage 2 mock: updatePipelineNode
		const updateNodeSpy = vi.spyOn(pipelinesApi, 'updatePipelineNode').mockResolvedValue({
			success: true,
			data: {}
		} as any)

		const result = await store.saveGraph()

		expect(result).toBe(true)
		expect(store.isDirty).toBe(false)
		expect(store.error).toBeNull()
		expect(store.successMessage).toBe(
			'Pipeline graph and all node contents saved successfully!'
		)

		// Verify Stage 1 was called
		expect(graphSpy).toHaveBeenCalledTimes(1)

		// Verify Stage 2 called getPipelineNode and updatePipelineNode for all 3 nodes
		expect(getNodeSpy).toHaveBeenCalledTimes(3)
		expect(updateNodeSpy).toHaveBeenCalledTimes(3)

		// Inspect source node update payload:
		// item-uuid-2 was added, item-uuid-old was removed
		const sourceCall = updateNodeSpy.mock.calls.find(
			(call) => call[0].node_id === 'source-node-1'
		)
		expect(sourceCall).toBeDefined()
		expect(sourceCall![1]).toEqual({
			last_updated: '2026-10-22T10:01:00Z',
			kind: 'source',
			data: {
				add: ['item-uuid-2'],
				remove: ['item-uuid-old']
			}
		})

		// Inspect processor group update payload:
		const procCall = updateNodeSpy.mock.calls.find(
			(call) => call[0].node_id === 'proc-group-node-2'
		)
		expect(procCall).toBeDefined()
		expect(procCall![1]).toEqual({
			last_updated: '2026-10-22T10:02:00Z',
			kind: 'processor_group',
			data: {
				processors: [
					{
						processor_id: 'proc-uuid-1',
						name: 'Drop Passwords',
						adm_status: 1,
						config: {
							type: 'drop_attributes',
							config: {
								keys: {
									record: ['password']
								}
							}
						}
					},
					{
						processor_id: 'proc-uuid-2',
						name: 'Inactive Filter',
						adm_status: 0,
						config: {
							type: 'null'
						}
					}
				]
			}
		})

		// Inspect sink update payload:
		const sinkCall = updateNodeSpy.mock.calls.find((call) => call[0].node_id === 'sink-node-3')
		expect(sinkCall).toBeDefined()
		expect(sinkCall![1]).toEqual({
			last_updated: '2026-10-22T10:03:00Z',
			kind: 'sink',
			data: {
				sink_id: 'sink-uuid-99'
			}
		})
	})

	it('empties sink node with data: null when sinkId is not set', async () => {
		store.nodes = [
			{
				id: 'empty-sink-node',
				type: 'sink',
				label: 'Unassigned Sink',
				position: { x: 500, y: 150 },
				data: {
					node_id: 'empty-sink-node',
					sinkId: undefined
				}
			}
		] as any

		vi.spyOn(pipelinesApi, 'updatePipelineGraph').mockResolvedValue({
			success: true,
			data: { last_updated: '2026-10-22T10:00:00Z' }
		} as any)

		vi.spyOn(pipelinesApi, 'getPipelineNode').mockResolvedValue({
			success: true,
			data: {
				node_id: 'empty-sink-node',
				last_updated: '2026-10-22T10:05:00Z',
				content: { kind: 'sink', data: null }
			}
		} as any)

		const updateNodeSpy = vi.spyOn(pipelinesApi, 'updatePipelineNode').mockResolvedValue({
			success: true,
			data: {}
		} as any)

		const result = await store.saveGraph()
		expect(result).toBe(true)

		const sinkCall = updateNodeSpy.mock.calls[0]
		expect(sinkCall[1]).toEqual({
			last_updated: '2026-10-22T10:05:00Z',
			kind: 'sink',
			data: null
		})
	})

	it('handles failure when a node content update fails via Promise.allSettled and flags isDirty', async () => {
		store.nodes = [
			{
				id: 'node-success',
				type: 'sink',
				label: 'Sink Success',
				position: { x: 100, y: 100 },
				data: { node_id: 'node-success', sinkId: 'sink-1' }
			},
			{
				id: 'node-fail',
				type: 'processor',
				label: 'Processor Faulty',
				position: { x: 300, y: 100 },
				data: { node_id: 'node-fail', processors: [] }
			}
		] as any

		vi.spyOn(pipelinesApi, 'updatePipelineGraph').mockResolvedValue({
			success: true,
			data: { last_updated: '2026-10-22T10:00:00Z' }
		} as any)

		vi.spyOn(pipelinesApi, 'getPipelineNode').mockImplementation((params) => {
			if (params.node_id === 'node-fail') {
				// Simulate read failure
				return Promise.resolve({ success: false, data: null } as any)
			}
			return Promise.resolve({
				success: true,
				data: {
					node_id: 'node-success',
					last_updated: '2026-10-22T10:10:00Z',
					content: { kind: 'sink', data: null }
				}
			} as any)
		})

		vi.spyOn(pipelinesApi, 'updatePipelineNode').mockResolvedValue({
			success: true,
			data: {}
		} as any)

		const result = await store.saveGraph()

		expect(result).toBe(false)
		expect(store.isDirty).toBe(true)
		expect(store.error).toContain('Processor Faulty')
		expect(store.error).toContain('Failed to read node before saving')
	})

	it('auto-repairs processor IDs when a processor has an ID from an existing cluster processor', async () => {
		const collidingClusterId = '01a16240-2860-7913-a282-45a5e923db01'
		vi.spyOn(processorsApi, 'get').mockResolvedValue({
			success: true,
			data: {
				processors: [
					{
						processor_id: collidingClusterId,
						name: 'Cluster Processor'
					}
				]
			}
		} as any)

		store.nodes = [
			{
				id: 'proc-group-node-1',
				type: 'processor',
				label: 'Group',
				position: { x: 100, y: 100 },
				data: {
					node_id: 'proc-group-node-1',
					processors: [
						{
							processor_id: collidingClusterId,
							name: 'Cloned Processor',
							module: 'null',
							status: true
						}
					]
				}
			}
		] as any

		vi.spyOn(pipelinesApi, 'updatePipelineGraph').mockResolvedValue({
			success: true,
			data: { last_updated: '2026-10-22T10:00:00Z' }
		} as any)

		vi.spyOn(pipelinesApi, 'getPipelineNode').mockResolvedValue({
			success: true,
			data: {
				node_id: 'proc-group-node-1',
				last_updated: '2026-10-22T10:10:00Z',
				content: {
					kind: 'processor_group',
					data: {
						processors: [] // Empty in backend
					}
				}
			}
		} as any)

		let sentPayload: any = null
		vi.spyOn(pipelinesApi, 'updatePipelineNode').mockImplementation((_params, payload) => {
			sentPayload = payload
			return Promise.resolve({ success: true, data: {} } as any)
		})

		const result = await store.saveGraph()

		expect(result).toBe(true)
		expect(sentPayload).not.toBeNull()
		const sentProcessorId = sentPayload.data.processors[0].processor_id
		expect(sentProcessorId).not.toBe(collidingClusterId)
		expect(sentProcessorId).toMatch(
			/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i
		)
		expect(store.nodes[0].data.processors[0].processor_id).toBe(sentProcessorId)
	})

	it('automatically retries with fresh UUIDs when backend returns Processor IDs already used error', async () => {
		const collidedId = '01a16240-2860-7913-a282-45b5ed0645b9'
		store.nodes = [
			{
				id: 'proc-group-node-2',
				type: 'processor',
				label: 'Group With Collision',
				position: { x: 100, y: 100 },
				data: {
					node_id: 'proc-group-node-2',
					processors: [
						{
							processor_id: collidedId,
							name: 'Colliding Item',
							module: 'null',
							status: true
						}
					]
				}
			}
		] as any

		vi.spyOn(pipelinesApi, 'updatePipelineGraph').mockResolvedValue({
			success: true,
			data: { last_updated: '2026-10-22T10:00:00Z' }
		} as any)

		vi.spyOn(pipelinesApi, 'getPipelineNode').mockResolvedValue({
			success: true,
			data: {
				node_id: 'proc-group-node-2',
				last_updated: '2026-10-22T10:10:00Z',
				content: {
					kind: 'processor_group',
					data: { processors: [] }
				}
			}
		} as any)

		let callCount = 0
		let lastSentPayload: any = null
		vi.spyOn(pipelinesApi, 'updatePipelineNode').mockImplementation((_params, payload) => {
			callCount++
			lastSentPayload = payload
			if (callCount === 1) {
				return Promise.resolve({
					status_code: 400,
					success: false,
					error: {
						title: 'Validation error',
						detail: `Processor IDs already used: ${collidedId}`
					}
				} as any)
			}
			return Promise.resolve({ success: true, data: {} } as any)
		})

		const result = await store.saveGraph()

		expect(result).toBe(true)
		expect(callCount).toBe(2)
		expect(lastSentPayload.data.processors[0].processor_id).not.toBe(collidedId)
		expect(store.nodes[0].data.processors[0].processor_id).toBe(
			lastSentPayload.data.processors[0].processor_id
		)
	})

	function makeProcessorNode(id: string, name: string, processors: any[] = [], x = 200) {
		return {
			id,
			type: 'processor',
			label: name,
			position: { x, y: 150 },
			data: {
				node_id: id,
				name,
				processors,
				processCount: processors.length,
				activeCount: processors.filter((p) => p.status !== false).length
			}
		}
	}

	function mockBackendGraphNodes(nodeIds: string[]) {
		store.canvasGraph = {
			graph: {
				nodes: nodeIds.map((node_id) => ({ node_id })),
				relations: []
			},
			last_updated: '2026-10-22T10:00:00Z'
		} as any
	}

	it('triggers full saveGraph() when creating a processor in modal and clears isDirty on success', async () => {
		store.isDirty = false
		store.nodes = [makeProcessorNode('proc-group-node-1', 'Transform Group')] as any

		const graphSpy = vi.spyOn(pipelinesApi, 'updatePipelineGraph').mockResolvedValue({
			success: true,
			data: { last_updated: '2026-10-22T10:00:00Z' }
		} as any)
		vi.spyOn(pipelinesApi, 'getPipelineNode').mockResolvedValue({
			success: true,
			data: {
				node_id: 'proc-group-node-1',
				last_updated: '2026-10-22T10:10:00Z',
				content: { kind: 'processor_group', data: { processors: [] } }
			}
		} as any)
		const nodeSpy = vi
			.spyOn(pipelinesApi, 'updatePipelineNode')
			.mockResolvedValue({ success: true, data: {} } as any)

		store.openCreateProcessorFormModal('proc-group-node-1')
		const processorStore = useProcessorStore()
		expect(processorStore.customSubmitHandler).toBeTypeOf('function')

		const result = await processorStore.customSubmitHandler!({
			name: 'Critical Logs OPL',
			adm_status: 1,
			config: {
				type: 'transform',
				module: 'transform',
				config: {
					opl: 'logs | where severity_number >= 17'
				}
			}
		})

		expect(result).toBe(true)
		expect(graphSpy).toHaveBeenCalledTimes(1)
		expect(nodeSpy).toHaveBeenCalledTimes(1)
		expect(store.isDirty).toBe(false)
		expect(store.nodes[0].data.processors).toHaveLength(1)
		expect(store.nodes[0].data.processors[0].name).toBe('Critical Logs OPL')
		expect(store.nodes[0].data.processCount).toBe(1)
	})

	it('rolls back local processor change and returns false when saveGraph() fails on create or edit', async () => {
		const existingProc = {
			processor_id: 'proc-existing-1',
			name: 'Existing Transform',
			module: 'transform',
			status: true,
			config: {
				type: 'transform',
				module: 'transform',
				config: { opl: 'logs | where severity_number >= 10' }
			}
		}

		store.isDirty = false
		store.nodes = [
			makeProcessorNode('proc-group-node-1', 'Transform Group', [existingProc])
		] as any

		vi.spyOn(pipelinesApi, 'updatePipelineGraph').mockResolvedValue({
			success: true,
			data: { last_updated: '2026-10-22T10:00:00Z' }
		} as any)
		vi.spyOn(pipelinesApi, 'getPipelineNode').mockResolvedValue({
			success: true,
			data: {
				node_id: 'proc-group-node-1',
				last_updated: '2026-10-22T10:10:00Z',
				content: { kind: 'processor_group', data: { processors: [existingProc] } }
			}
		} as any)
		vi.spyOn(pipelinesApi, 'updatePipelineNode').mockRejectedValue(
			new Error('invalid OPL: line 1, column 8')
		)

		const processorStore = useProcessorStore()

		// 1. Create failure should roll back to only existingProc
		store.openCreateProcessorFormModal('proc-group-node-1')
		const createResult = await processorStore.customSubmitHandler!({
			name: 'Broken OPL',
			adm_status: 1,
			config: { type: 'transform', module: 'transform', config: { opl: 'logs | where' } }
		})

		expect(createResult).toBe(false)
		expect(store.nodes[0].data.processors).toEqual([existingProc])
		expect(store.nodes[0].data.processCount).toBe(1)
		expect(store.isDirty).toBe(false)

		// 2. Edit failure should roll back existingProc config
		await store.openEditProcessorFormModal('proc-group-node-1', existingProc as any)
		const editResult = await processorStore.customSubmitHandler!({
			name: 'Edited Broken OPL',
			adm_status: 1,
			config: { type: 'transform', module: 'transform', config: { opl: 'logs | where' } }
		})

		expect(editResult).toBe(false)
		expect(store.nodes[0].data.processors).toEqual([existingProc])
		expect(store.nodes[0].data.processors[0].name).toBe('Existing Transform')
		expect(store.isDirty).toBe(false)
	})

	it('does not roll back target node when target succeeds and another node fails during modal save', async () => {
		store.isDirty = false
		store.nodes = [
			makeProcessorNode('proc-group-target', 'Target Group'),
			{
				id: 'sink-node-other',
				type: 'sink',
				label: 'Other Sink',
				position: { x: 450, y: 150 },
				data: {
					node_id: 'sink-node-other',
					sinkId: 'sink-1'
				}
			}
		] as any

		vi.spyOn(pipelinesApi, 'updatePipelineGraph').mockResolvedValue({
			success: true,
			data: { last_updated: '2026-10-22T10:00:00Z' }
		} as any)
		vi.spyOn(pipelinesApi, 'getPipelineNode').mockImplementation((params) => {
			if (params.node_id === 'sink-node-other') {
				return Promise.resolve({ success: false, data: null } as any)
			}
			return Promise.resolve({
				success: true,
				data: {
					node_id: 'proc-group-target',
					last_updated: '2026-10-22T10:10:00Z',
					content: { kind: 'processor_group', data: { processors: [] } }
				}
			} as any)
		})
		vi.spyOn(pipelinesApi, 'updatePipelineNode').mockResolvedValue({
			success: true,
			data: {}
		} as any)

		const appStore = useAppStore()
		appStore.notificationItems = []

		store.openCreateProcessorFormModal('proc-group-target')
		const processorStore = useProcessorStore()
		const result = await processorStore.customSubmitHandler!({
			name: 'Saved OPL Processor',
			adm_status: 1,
			config: {
				type: 'transform',
				module: 'transform',
				config: { opl: 'logs | where severity_number >= 17' }
			}
		})

		expect(result).toBe(true)
		expect(store.nodes[0].data.processors).toHaveLength(1)
		expect(store.nodes[0].data.processors[0].name).toBe('Saved OPL Processor')
		expect(store.isDirty).toBe(true)
		expect(appStore.notificationItems.some((n: any) => n.type === 'error')).toBe(true)
	})

	it('fetches cluster processors lazily at most once per saveGraph() and skips when editing existing processors', async () => {
		const existingId = '01a16240-2860-7913-a282-45b5ed0645a1'
		const newId1 = '01a16240-2860-7913-a282-45b5ed0645a2'
		const newId2 = '01a16240-2860-7913-a282-45b5ed0645a3'

		const procGetSpy = vi.spyOn(processorsApi, 'get').mockResolvedValue({
			success: true,
			data: { processors: [], total: 0 }
		} as any)

		vi.spyOn(pipelinesApi, 'updatePipelineGraph').mockResolvedValue({
			success: true,
			data: { last_updated: '2026-10-22T10:00:00Z' }
		} as any)
		vi.spyOn(pipelinesApi, 'updatePipelineNode').mockResolvedValue({
			success: true,
			data: {}
		} as any)

		// Case 1: Editing existing processor (already in currentBackendIds) -> 0 calls to processorsApi.get
		store.nodes = [
			makeProcessorNode('group-1', 'Group 1', [
				{ processor_id: existingId, name: 'Existing', module: 'null' }
			])
		] as any
		vi.spyOn(pipelinesApi, 'getPipelineNode').mockResolvedValue({
			success: true,
			data: {
				node_id: 'group-1',
				last_updated: '2026-10-22T10:00:00Z',
				content: {
					kind: 'processor_group',
					data: { processors: [{ processor_id: existingId }] }
				}
			}
		} as any)

		await store.saveGraph()
		expect(procGetSpy).not.toHaveBeenCalled()

		// Case 2: Multiple processor groups with new processors -> only 1 shared processorsApi.get call
		store.nodes = [
			makeProcessorNode(
				'group-1',
				'Group 1',
				[{ processor_id: newId1, name: 'New 1', module: 'null' }],
				100
			),
			makeProcessorNode(
				'group-2',
				'Group 2',
				[{ processor_id: newId2, name: 'New 2', module: 'null' }],
				300
			)
		] as any
		vi.spyOn(pipelinesApi, 'getPipelineNode').mockResolvedValue({
			success: true,
			data: {
				last_updated: '2026-10-22T10:00:00Z',
				content: { kind: 'processor_group', data: { processors: [] } }
			}
		} as any)

		await store.saveGraph()
		expect(procGetSpy).toHaveBeenCalledTimes(1)
	})

	it('saves per-node via saveNodeContent() without calling updatePipelineGraph when node exists in backend', async () => {
		store.isDirty = false
		mockBackendGraphNodes(['proc-group-existing', 'sink-other'])
		store.nodes = [
			makeProcessorNode('proc-group-existing', 'Existing Group'),
			{
				id: 'sink-other',
				type: 'sink',
				label: 'Other Sink',
				position: { x: 450, y: 150 },
				data: {
					node_id: 'sink-other',
					sinkId: 'sink-1'
				}
			}
		] as any

		const graphSpy = vi.spyOn(pipelinesApi, 'updatePipelineGraph')
		const getNodeSpy = vi.spyOn(pipelinesApi, 'getPipelineNode').mockResolvedValue({
			success: true,
			data: {
				node_id: 'proc-group-existing',
				last_updated: '2026-10-22T10:10:00Z',
				content: { kind: 'processor_group', data: { processors: [] } }
			}
		} as any)
		const updateNodeSpy = vi
			.spyOn(pipelinesApi, 'updatePipelineNode')
			.mockResolvedValue({ success: true, data: {} } as any)

		store.openCreateProcessorFormModal('proc-group-existing')
		const processorStore = useProcessorStore()
		const result = await processorStore.customSubmitHandler!({
			name: 'Per-Node OPL',
			adm_status: 1,
			config: {
				type: 'transform',
				module: 'transform',
				config: { opl: 'logs | where severity_number >= 17' }
			}
		})

		expect(result).toBe(true)
		expect(graphSpy).not.toHaveBeenCalled()
		expect(getNodeSpy).toHaveBeenCalledTimes(1)
		expect(updateNodeSpy).toHaveBeenCalledTimes(1)
		expect(updateNodeSpy.mock.calls[0][0].node_id).toBe('proc-group-existing')
		expect(store.isDirty).toBe(false)
		expect(store.nodes[0].data.processors).toHaveLength(1)

		// Preserves existing isDirty=true when canvas already had unsaved layout/other changes
		store.isDirty = true
		const existingItem = store.nodes[0].data.processors[0]
		await store.openEditProcessorFormModal('proc-group-existing', existingItem)
		const editResult = await processorStore.customSubmitHandler!({
			name: 'Per-Node OPL Updated',
			adm_status: 1,
			config: {
				type: 'transform',
				module: 'transform',
				config: { opl: 'logs | where severity_number >= 18' }
			}
		})

		expect(editResult).toBe(true)
		expect(graphSpy).not.toHaveBeenCalled()
		expect(store.isDirty).toBe(true)
		expect(store.nodes[0].data.processors[0].name).toBe('Per-Node OPL Updated')
	})

	it('rolls back and notifies on per-node save failure when node exists in backend', async () => {
		const existingProc = {
			processor_id: 'proc-existing-1',
			name: 'Existing Transform',
			module: 'transform',
			status: true,
			config: {
				type: 'transform',
				module: 'transform',
				config: { opl: 'logs | where severity_number >= 10' }
			}
		}

		store.isDirty = true
		mockBackendGraphNodes(['proc-group-existing'])
		store.nodes = [
			makeProcessorNode('proc-group-existing', 'Existing Group', [existingProc])
		] as any

		const graphSpy = vi.spyOn(pipelinesApi, 'updatePipelineGraph')
		vi.spyOn(pipelinesApi, 'getPipelineNode').mockResolvedValue({
			success: true,
			data: {
				node_id: 'proc-group-existing',
				last_updated: '2026-10-22T10:10:00Z',
				content: { kind: 'processor_group', data: { processors: [existingProc] } }
			}
		} as any)
		vi.spyOn(pipelinesApi, 'updatePipelineNode').mockRejectedValue(
			new Error('invalid OPL: line 1, column 8')
		)

		const appStore = useAppStore()
		appStore.notificationItems = []

		await store.openEditProcessorFormModal('proc-group-existing', existingProc as any)
		const processorStore = useProcessorStore()
		const result = await processorStore.customSubmitHandler!({
			name: 'Broken Edit',
			adm_status: 1,
			config: {
				type: 'transform',
				module: 'transform',
				config: { opl: 'logs | where' }
			}
		})

		expect(result).toBe(false)
		expect(graphSpy).not.toHaveBeenCalled()
		expect(store.nodes[0].data.processors).toEqual([existingProc])
		expect(store.isDirty).toBe(true)
		expect(appStore.notificationItems.some((n: any) => n.type === 'error')).toBe(true)
	})
})

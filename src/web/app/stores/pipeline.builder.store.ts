// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { defineStore } from 'pinia'
import { computed, reactive, ref } from 'vue'
import pipelineManagementApi from '@/app/api/pipeline.management.api'
import hostsApi from '@/app/api/hosts.api'
import sinksApi from '@/app/api/sinks.api'
import itemsApi from '@/app/api/items.api'
import processorsApi from '@/app/api/processors.api'
import appHelper from '@/app/helpers/app.helper'
import { generateUuidV7 } from '@/app/helpers/uuid.helper'
import { useProcessorStore } from '@/stores/processor.store'
import { FormMode } from '@@/shared/types/app.type'
import type { IProcessor } from '@@/shared/types/processor.type'
import { MarkerType } from '@vue-flow/core'
import dagre from '@dagrejs/dagre'
import type { INode } from '@@/shared/types/canvas.type'
import type {
	GetPipelineManagementGraphResponse,
	PipelineManagementDetail,
	UpdatePipelineManagementGraphRequest,
	UpdatePipelineManagementNodeRequest
} from '@@/shared/generated/patrolliAPI.schemas.type'

export function getProcessorModuleBadgeClass(moduleName: string, withBorder: boolean = true) {
	const mod = (moduleName || '').toLowerCase()
	if (mod.includes('filter')) {
		return withBorder
			? 'bg-sky-50 dark:bg-sky-950/60 text-[#0284C7] dark:text-sky-300 border-sky-200 dark:border-sky-800'
			: 'bg-sky-50 dark:bg-sky-950/60 text-[#0284C7] dark:text-sky-300'
	}
	if (mod.includes('drop')) {
		return withBorder
			? 'bg-rose-50 dark:bg-rose-950/60 text-rose-600 dark:text-rose-300 border-rose-200 dark:border-rose-800'
			: 'bg-rose-50 dark:bg-rose-950/60 text-rose-600 dark:text-rose-300'
	}
	if (mod.includes('add')) {
		return withBorder
			? 'bg-emerald-50 dark:bg-emerald-950/60 text-emerald-600 dark:text-emerald-300 border-emerald-200 dark:border-emerald-800'
			: 'bg-emerald-50 dark:bg-emerald-950/60 text-emerald-600 dark:text-emerald-300'
	}
	if (mod.includes('rename')) {
		return withBorder
			? 'bg-amber-50 dark:bg-amber-950/60 text-amber-600 dark:text-amber-300 border-amber-200 dark:border-amber-800'
			: 'bg-amber-50 dark:bg-amber-950/60 text-amber-600 dark:text-amber-300'
	}
	if (mod.includes('lookup')) {
		return withBorder
			? 'bg-purple-50 dark:bg-purple-950/60 text-purple-600 dark:text-purple-300 border-purple-200 dark:border-purple-800'
			: 'bg-purple-50 dark:bg-purple-950/60 text-purple-600 dark:text-purple-300'
	}
	return withBorder
		? 'bg-slate-100 dark:bg-slate-800 text-slate-600 dark:text-slate-300 border-slate-200 dark:border-slate-700'
		: 'bg-slate-100 dark:bg-slate-800 text-slate-600 dark:text-slate-300'
}

export interface ISourceNodeData {
	id?: string
	node_id?: string
	title: string
	hostCount: number
	itemCount: number
	item_count?: number
	hosts: string[]
	items: string[]
	health: {
		healthy: number
		warning: number
		error: number
	}
}

export interface IGroupProcessorItem {
	processor_id: string
	source_processor_id?: string
	name: string
	module: string
	status?: boolean
	adm_status?: number
	item_ids?: string[]
	type?: string
	config?: any
}

export interface IProcessorNodeData {
	id: string
	node_id?: string
	name: string
	type: string
	processCount: number
	itemCount?: number
	activeCount?: number
	processors?: IGroupProcessorItem[]
	config?: any
}

export interface ISinkNodeData {
	id: string
	node_id?: string
	sinkId?: string
	name: string
	sinkName?: string
	type: string
}

export const usePipelineBuildStore = defineStore(
	'pipelineBuild',
	() => {
		const canvasKey = ref<string>(Math.random().toString(36).substring(2, 9))
		const pipelineId = ref<string | null>(null)
		const canvasId = ref<string | null>(null)
		const clusterUrl = ref<string | null>(null)

		const pipelineDetail = ref<PipelineManagementDetail | null>(null)
		const canvasGraph = ref<GetPipelineManagementGraphResponse | null>(null)

		const nodes = ref<any[]>([])
		const edges = ref<any[]>([])
		const selectedNode = ref<any>()
		const isCanvasEmpty = computed(() => nodes.value.length === 0)

		const isDirty = ref(false)
		const loading = reactive({
			canvas: false,
			saving: false,
			detail: false
		})
		const error = ref<string | null>(null)
		const successMessage = ref<string | null>(null)

		// Available lists for adding/editing nodes
		const availableHosts = ref<any[]>([])
		const availableSinks = ref<any[]>([])
		const pipelineItems = ref<any[]>([])
		const loadingItems = ref(false)

		const edgeItemModal = ref<{
			isOpen: boolean
			edgeId?: string
			sourceId?: string
			targetId?: string
		}>({
			isOpen: false
		})

		const previewDataModal = ref<{
			isOpen: boolean
			nodeId?: string
			processorId?: string
		}>({
			isOpen: false
		})

		async function fetchPipelineItems() {
			const hostIds = pipelineDetail.value?.host_ids
			if (!hostIds || hostIds.length === 0) {
				pipelineItems.value = []
				return
			}
			loadingItems.value = true
			const cUrls = clusterUrl.value ? [clusterUrl.value] : undefined
			try {
				const res = await itemsApi
					.getItems({ host_ids: hostIds, limit: 100 }, cUrls)
					.catch(() => undefined)
				pipelineItems.value = res?.data?.items || []
			} catch (err) {
				console.error('Failed to fetch pipeline items:', err)
			} finally {
				loadingItems.value = false
			}
		}

		// Legacy compatibility for configuration/pipeline-builder
		const itemPagination = reactive({
			rows: 10,
			currentPage: 1,
			totalRecords: 0,
			reachEnd: false
		})
		const itemFilter = reactive<any>({})
		const items = ref<any[]>([])
		const itemLoading = reactive({ table: false })
		const itemError = ref<string | null>(null)

		async function fetchItems(_isSearch?: boolean, _selected?: string[]) {}
		async function fetchGraph() {}
		async function updateCanvas() {}

		function markDirty() {
			isDirty.value = true
		}

		async function loadBuilder(pId: string, cId: string, cluster?: string) {
			pipelineId.value = pId
			canvasId.value = cId
			clusterUrl.value = cluster || null
			loading.canvas = true
			error.value = null
			isDirty.value = false
			selectedNode.value = undefined

			const cUrls = clusterUrl.value ? [clusterUrl.value] : undefined

			try {
				// 1. Fetch Pipeline Detail
				const detailRes = await pipelineManagementApi.getPipelineById(pId, cUrls)
				if (detailRes?.success && detailRes.data) {
					pipelineDetail.value = detailRes.data
					await fetchPipelineItems()
				}

				// Fetch host & sink metadata for display
				const [hostsRes, sinksRes] = await Promise.all([
					hostsApi.getHosts({ limit: 100 }, cUrls).catch(() => undefined),
					sinksApi.getSinks({ limit: 100 }, cUrls).catch(() => undefined)
				])
				availableHosts.value = hostsRes?.data?.hosts || []
				availableSinks.value = sinksRes?.data?.sinks || []

				// 2. Fetch Canvas Graph
				if (cId && cId !== 'default') {
					const graphRes = await pipelineManagementApi.getPipelineGraph(
						{ pipeline_management_id: pId, canvas_id: cId },
						cUrls
					)
					if (graphRes?.success && graphRes.data) {
						canvasGraph.value = graphRes.data

						// Fetch source node items so edge connections and source node reflect saved items
						const sourceNode = graphRes.data.graph?.nodes?.find(
							(n: any) => n.kind === 'source'
						)
						if (sourceNode?.node_id) {
							try {
								const sourceNodeRes = await pipelineManagementApi.getPipelineNode(
									{
										pipeline_management_id: pId,
										canvas_id: cId,
										node_id: sourceNode.node_id,
										query: { limit: 100 }
									},
									cUrls
								)
								const sourceItems =
									(sourceNodeRes?.data as any)?.content?.data?.items ||
									(sourceNodeRes?.data as any)?.data?.items ||
									[]
								if (!sourceNode.data) (sourceNode as any).data = {}
								;(sourceNode.data as any).items = sourceItems
								if (
									typeof (sourceNodeRes?.data as any)?.content?.data?.total ===
									'number'
								) {
									;(sourceNode.data as any).item_count = (
										sourceNodeRes?.data as any
									).content.data.total
								} else if (sourceItems.length > 0) {
									;(sourceNode.data as any).item_count = sourceItems.length
								}
							} catch (e) {
								console.error('Failed to fetch source node items in loadCanvas:', e)
							}
						}

						// Fetch processor group details so processor status (adm_status) and config are loaded
						const procNodes =
							graphRes.data.graph?.nodes?.filter(
								(n: any) => n.kind === 'processor_group'
							) || []
						if (procNodes.length > 0) {
							await Promise.allSettled(
								procNodes.map(async (pn: any) => {
									if (!pn?.node_id) return
									try {
										const pnRes = await pipelineManagementApi.getPipelineNode(
											{
												pipeline_management_id: pId,
												canvas_id: cId,
												node_id: pn.node_id
											},
											cUrls
										)
										const procList =
											(pnRes?.data as any)?.data?.processors ||
											(pnRes?.data as any)?.content?.data?.processors ||
											(pnRes?.data as any)?.processors
										if (procList && Array.isArray(procList)) {
											if (!pn.data) pn.data = {}
											;(pn.data as any).processors = procList
										}
									} catch (e) {
										console.error(
											'Failed to fetch processor group node content in loadCanvas:',
											e
										)
									}
								})
							)
						}
					}
				}

				// 3. Reconcile Nodes & Edges from Backend Graph or Fallback Empty States
				buildGraphRepresentation()
			} catch (err: any) {
				error.value = err.message || 'Failed to load pipeline builder canvas'
			} finally {
				loading.canvas = false
			}
		}

		function extractItemIds(items: any[]): string[] {
			if (!Array.isArray(items)) return []
			return items
				.map((i: any) => (typeof i === 'string' ? i : i?.item_id || i?.id))
				.filter((id): id is string => typeof id === 'string' && id.length > 0)
		}

		function createBuilderEdge(params: {
			id: string
			source: string
			target: string
			sourceHandle?: string
			targetHandle?: string
			data?: Record<string, any>
		}) {
			return {
				id: params.id,
				source: params.source,
				sourceHandle: params.sourceHandle || 'right',
				target: params.target,
				targetHandle: params.targetHandle || 'left',
				type: 'custom',
				data: params.data || {},
				markerEnd: {
					type: MarkerType.ArrowClosed,
					color: '#0284C7',
					width: 14,
					height: 14
				},
				style: { stroke: '#0284C7', strokeWidth: 2.5 }
			}
		}

		function getSourceActiveItemIds(sourceId: string): string[] {
			const outgoingEdges = edges.value.filter(
				(e: any) =>
					e.source === sourceId ||
					e.source === `source-${sourceId}` ||
					(e.source && sourceId.includes(e.source))
			)
			const allIds = new Set<string>()
			outgoingEdges.forEach((e: any) => {
				const ids = e.data?.selectedItemIds || []
				ids.forEach((id: string) => allIds.add(id))
			})
			return Array.from(allIds)
		}

		function buildGraphRepresentation() {
			selectedNode.value = undefined
			const existingNodes = canvasGraph.value?.graph?.nodes || []
			const existingRelations = canvasGraph.value?.graph?.relations || []

			// If canvas has existing saved nodes, map them directly
			if (existingNodes.length > 0) {
				const sourceNodeMeta: any = existingNodes.find((gn: any) => gn.kind === 'source')
				nodes.value = existingNodes.map((gn: any) => {
					let nType = 'processor'
					let label = 'Processor'
					let nodeData: any = {}
					const nodeId = gn.node_id || generateUuidV7()

					if (gn.kind === 'source') {
						nType = 'source'
						label = 'Source'
						const hostNames = (pipelineDetail.value?.host_ids || []).map((hid) => {
							const found = availableHosts.value.find((h) => h.host_id === hid)
							return found?.name || hid
						})
						const count =
							typeof gn.data?.item_count === 'number'
								? gn.data.item_count
								: Array.isArray(gn.data?.items)
									? gn.data.items.length
									: 0
						nodeData = {
							id: nodeId,
							node_id: nodeId,
							title: 'Source',
							hostCount: pipelineDetail.value?.host_ids?.length || 0,
							itemCount: count,
							item_count: count,
							hosts: hostNames,
							items: gn.data?.items || [],
							health: {
								healthy: Math.max(0, pipelineDetail.value?.host_ids?.length || 0),
								warning: 0,
								error: 0
							}
						} as ISourceNodeData
					} else if (gn.kind === 'sink') {
						nType = 'sink'
						const sinkId = (gn.data as any)?.sink_id
						const sinkMeta = availableSinks.value.find((s) => s.sink_id === sinkId)
						label = 'Sink Target'
						nodeData = {
							id: nodeId,
							node_id: nodeId,
							sinkId: sinkId,
							name: 'Sink Target',
							sinkName: sinkMeta?.name,
							type: sinkMeta?.type || 'kafka'
						} as ISinkNodeData
					} else {
						nType = 'processor'
						label = (gn.data as any)?.name || 'Processor Group'
						const rawProcessors = (gn.data as any)?.processors || []
						const normalizedProcessors: IGroupProcessorItem[] = Array.isArray(
							rawProcessors
						)
							? rawProcessors.map((p: any, idx: number) => {
									const isStatusActive =
										p.status !== undefined
											? Boolean(p.status)
											: p.adm_status !== undefined
												? Number(p.adm_status) === 1
												: true
									return {
										processor_id: p.processor_id || generateUuidV7(),
										name: p.name || `Processor ${idx + 1}`,
										module:
											p.module ||
											p.config?.type ||
											p.config?.module ||
											'null',
										status: isStatusActive,
										adm_status:
											p.adm_status !== undefined
												? Number(p.adm_status)
												: isStatusActive
													? 1
													: 2,
										item_ids: p.item_ids || [],
										type: p.type || 'Transform',
										config: p.config
									}
								})
							: []
						const activeCount = normalizedProcessors.filter(
							(p) => p.status !== false
						).length
						nodeData = {
							id: nodeId,
							node_id: nodeId,
							name: label,
							type: 'Transform',
							processors: normalizedProcessors,
							processCount: normalizedProcessors.length,
							activeCount
						} as IProcessorNodeData
					}

					return {
						id: nodeId,
						node_id: nodeId,
						type: nType,
						label,
						position: { x: gn.position_x, y: gn.position_y },
						data: nodeData
					} as INode
				})

				// Determine source item count and item IDs directly from API graph
				const sourceItemCount =
					typeof sourceNodeMeta?.data?.item_count === 'number'
						? sourceNodeMeta.data.item_count
						: Array.isArray(sourceNodeMeta?.data?.items)
							? sourceNodeMeta.data.items.length
							: typeof sourceNodeMeta?.data?.itemCount === 'number'
								? sourceNodeMeta.data.itemCount
								: 0
				const sourceItemIds = extractItemIds(sourceNodeMeta?.data?.items || [])

				const mappedEdges: any[] = existingRelations.map((rel: any, idx) => {
					const fromNodeMeta: any = existingNodes.find(
						(gn: any) => gn.node_id === rel.from_node_id
					)
					const isSourceToTarget =
						(fromNodeMeta && fromNodeMeta.kind === 'source') ||
						(sourceNodeMeta && rel.from_node_id === sourceNodeMeta.node_id) ||
						rel.from_node_id.includes('source')

					let lineLabel: string | undefined =
						rel.label ??
						rel.text ??
						rel.name ??
						rel.data?.label ??
						rel.data?.text ??
						(typeof rel.item_count === 'number' ? `${rel.item_count} Items` : undefined)

					let relItemIds: string[] = []
					if (rel.items || rel.data?.selectedItemIds) {
						relItemIds = extractItemIds(rel.items || rel.data?.selectedItemIds || [])
					} else if (isSourceToTarget) {
						relItemIds = sourceItemIds
					}

					if (!lineLabel && isSourceToTarget) {
						lineLabel = `${sourceItemCount} Items`
					}

					// Sync target processor node item count
					const targetNode = nodes.value.find((n: any) => n.id === rel.to_node_id)
					if (targetNode && targetNode.type === 'processor' && targetNode.data) {
						targetNode.data.itemCount = isSourceToTarget
							? sourceItemCount
							: relItemIds.length
					}

					return createBuilderEdge({
						id: `edge-${rel.from_node_id}-${rel.to_node_id}-${idx}`,
						source: rel.from_node_id,
						target: rel.to_node_id,
						data: isSourceToTarget
							? {
									label: lineLabel ?? '0 Items',
									selectedItemIds: relItemIds,
									showAddButton: false
								}
							: {
									...(lineLabel ? { label: lineLabel } : {}),
									showAddButton: true
								}
					})
				})
				edges.value = mappedEdges

				return
			}

			// Empty State: saat response api graph.nodes empty maka canvas menjadi empty state
			nodes.value = []
			edges.value = []
		}

		function addProcessorNode(x?: number, y?: number, name: string = 'Processor Group') {
			const nodeId = generateUuidV7()
			const procCount = nodes.value.filter((n: any) => n.type === 'processor').length
			const posX = x !== undefined ? x : 480 + procCount * 60
			const posY = y !== undefined ? y : 220 + (procCount % 2) * 40

			const newProcessor: INode = {
				id: nodeId,
				node_id: nodeId,
				type: 'processor',
				label: name,
				position: { x: posX, y: posY },
				data: {
					id: nodeId,
					node_id: nodeId,
					name,
					type: 'Transform',
					processors: [],
					processCount: 0,
					itemCount: 0,
					activeCount: 0
				} as IProcessorNodeData
			}

			nodes.value.push(newProcessor)
			markDirty()
			return newProcessor
		}

		function addSinkNode(x?: number, y?: number, sinkMeta?: any) {
			const nodeId = generateUuidV7()
			const meta = sinkMeta || null
			const type = meta ? meta.type || 'kafka' : 'No sink selected'
			const sinkCount = nodes.value.filter((n: any) => n.type === 'sink').length
			const posX = x !== undefined ? x : 740 + sinkCount * 60
			const posY = y !== undefined ? y : 220 + (sinkCount % 2) * 40

			const newSink: INode = {
				id: nodeId,
				node_id: nodeId,
				type: 'sink',
				label: 'Sink Target',
				position: { x: posX, y: posY },
				data: {
					id: nodeId,
					node_id: nodeId,
					sinkId: meta?.sink_id || undefined,
					name: 'Sink Target',
					sinkName: meta?.name || undefined,
					type
				} as ISinkNodeData
			}

			nodes.value.push(newSink)
			markDirty()
			return newSink
		}

		function addSourceNode(x?: number, y?: number) {
			const nodeId = generateUuidV7()
			const srcCount = nodes.value.filter((n: any) => n.type === 'source').length
			const posX = x !== undefined ? x : 80 + srcCount * 60
			const posY = y !== undefined ? y : 160 + (srcCount % 2) * 40

			const hosts = pipelineDetail.value?.host_ids || []
			const hostNames = hosts.map((hid) => {
				const found = availableHosts.value.find((h) => h.host_id === hid)
				return found?.name || hid
			})

			const newSource: INode = {
				id: nodeId,
				node_id: nodeId,
				type: 'source',
				label: 'Source',
				position: { x: posX, y: posY },
				data: {
					id: nodeId,
					node_id: nodeId,
					title: 'Source',
					hostCount: hosts.length || 0,
					itemCount: pipelineItems.value.length,
					hosts: hostNames,
					items: [],
					health: {
						healthy: Math.max(0, hosts.length),
						warning: 0,
						error: 0
					}
				} as ISourceNodeData
			}

			nodes.value.push(newSource)
			markDirty()
			return newSource
		}

		function selectSinkForNode(nodeId: string, sinkId: string) {
			const targetSink = availableSinks.value.find((s: any) => s.sink_id === sinkId)
			const targetNode = nodes.value.find((n: any) => n.id === nodeId)
			if (targetNode) {
				targetNode.label = 'Sink Target'
				if (!targetNode.data) targetNode.data = {}
				targetNode.data.sinkId = sinkId
				targetNode.data.name = 'Sink Target'
				targetNode.data.sinkName = targetSink?.name || undefined
				targetNode.data.type = targetSink?.type || 'kafka'
				if (selectedNode.value && selectedNode.value.id === nodeId) {
					selectedNode.value = { ...targetNode }
				}
				markDirty()
			}
		}

		function openEdgeItemSelector(edgeId: string) {
			const edge = edges.value.find((e: any) => e.id === edgeId)
			if (!edge) return

			edgeItemModal.value = {
				isOpen: true,
				edgeId,
				sourceId: edge.source,
				targetId: edge.target
			}
		}

		function closeEdgeItemSelector() {
			edgeItemModal.value = { isOpen: false }
		}

		function applyEdgeItems(edgeId: string, itemIds: string[]) {
			const edge = edges.value.find((e: any) => e.id === edgeId)
			if (edge) {
				if (!edge.data) edge.data = {}
				edge.data.selectedItemIds = itemIds
				edge.data.label = `${itemIds.length} Items`
				edge.data.showAddButton = false

				// Sync item count to target processor node
				const targetNode = nodes.value.find((n: any) => n.id === edge.target)
				if (targetNode && targetNode.type === 'processor') {
					if (!targetNode.data) targetNode.data = {}
					targetNode.data.itemCount = itemIds.length
				}

				// Sync source node items and all outgoing edges from source
				const sourceNode = nodes.value.find((n: any) => n.id === edge.source)
				if (sourceNode && sourceNode.type === 'source') {
					if (!sourceNode.data) sourceNode.data = {}
					sourceNode.data.items = itemIds
					sourceNode.data.itemCount = itemIds.length
					sourceNode.data.item_count = itemIds.length

					// Keep all branches from this source synchronized with the new selected items
					edges.value.forEach((e: any) => {
						if (e.source === sourceNode.id && e.id !== edge.id) {
							const otherTarget = nodes.value.find((n: any) => n.id === e.target)
							if (otherTarget?.type === 'processor') {
								if (!e.data) e.data = {}
								e.data.selectedItemIds = itemIds
								e.data.label = `${itemIds.length} Items`
								if (otherTarget.data) {
									otherTarget.data.itemCount = itemIds.length
								}
							}
						}
					})

					// Trigger PUT /api/pipeline_management/{id}/canvases/{cId}/nodes/{nodeId} only if node already exists in backend graph
					const existsInBackend = canvasGraph.value?.graph?.nodes?.some(
						(gn: any) => gn.node_id === (sourceNode.node_id || sourceNode.id)
					)
					if (pipelineId.value && canvasId.value && existsInBackend) {
						saveNodeContent(
							sourceNode,
							clusterUrl.value ? [clusterUrl.value] : undefined
						).catch((err) => {
							console.error(
								'Failed to auto-save source node items on applyEdgeItems:',
								err
							)
						})
					}
				}

				markDirty()
			}
			closeEdgeItemSelector()
		}

		function connectNodes(
			fromId: string,
			toId: string,
			sourceHandle: string = 'right',
			targetHandle: string = 'left'
		) {
			const edgeId = `edge-${fromId}-${toId}`
			if (!edges.value.some((e) => e.source === fromId && e.target === toId)) {
				const fromNode = nodes.value.find((n: any) => n.id === fromId)
				const toNode = nodes.value.find((n: any) => n.id === toId)
				const isSourceToProcessor =
					fromNode?.type === 'source' && toNode?.type === 'processor'

				let initialItemIds: string[] = []
				let initialLabel: string | undefined = undefined
				let shouldOpenModal = false

				if (isSourceToProcessor) {
					const existingSourceToProcEdges = edges.value.filter((e) => {
						if (e.source !== fromId) return false
						const targetNode = nodes.value.find((n: any) => n.id === e.target)
						return targetNode?.type === 'processor'
					})

					if (existingSourceToProcEdges.length > 0) {
						// Source already has a connection line to another processor group:
						// auto-match items from source node directly and do not open modal
						initialItemIds = extractItemIds(fromNode?.data?.items || [])
						const sourceCount =
							typeof fromNode?.data?.item_count === 'number'
								? fromNode.data.item_count
								: initialItemIds.length
						initialLabel = `${sourceCount} Items`
						shouldOpenModal = false

						if (toNode && toNode.data) {
							toNode.data.itemCount = sourceCount
						}
					} else {
						// First connection line from source to processor group:
						// start with 0 Items and open item selector modal
						initialItemIds = []
						initialLabel = '0 Items'
						shouldOpenModal = true
					}
				}

				edges.value.push(
					createBuilderEdge({
						id: edgeId,
						source: fromId,
						sourceHandle,
						target: toId,
						targetHandle,
						data: {
							showAddButton: !isSourceToProcessor,
							selectedItemIds: initialItemIds,
							label: initialLabel
						}
					})
				)
				markDirty()

				if (shouldOpenModal) {
					openEdgeItemSelector(edgeId)
				}
			}
			return edgeId
		}

		function insertProcessorBetween(edgeId: string, x?: number, y?: number) {
			const edgeIndex = edges.value.findIndex((e) => e.id === edgeId)
			if (edgeIndex === -1) return

			const oldEdge = edges.value[edgeIndex]
			const sourceNodeId = oldEdge.source
			const targetNodeId = oldEdge.target

			// 1. Buat node processor baru
			const newProc = addProcessorNode(x, y)

			// 2. Hapus edge lama yang dipotong agar tidak menembus node
			edges.value.splice(edgeIndex, 1)

			// 3. Hubungkan source ke newProc
			const fromNode = nodes.value.find((n) => n.id === sourceNodeId)
			const isFromSource = fromNode?.type === 'source' || sourceNodeId.includes('source')
			const hasOldItems =
				Array.isArray(oldEdge?.data?.selectedItemIds) &&
				oldEdge.data.selectedItemIds.length > 0

			let edgeItemIds: string[] = []
			let edgeLabel = '0 Items'

			if (hasOldItems) {
				edgeItemIds = oldEdge.data.selectedItemIds
				edgeLabel = oldEdge?.data?.label || `${edgeItemIds.length} Items`
			} else if (isFromSource) {
				// Old edge was connecting source directly to sink/other node without an item badge:
				// auto-match items from the source node
				const sourceItemIds = extractItemIds(fromNode?.data?.items || [])
				const sourceCount =
					typeof fromNode?.data?.item_count === 'number'
						? fromNode.data.item_count
						: sourceItemIds.length
				edgeItemIds = sourceItemIds
				edgeLabel = `${sourceCount} Items`
			}

			edges.value.push(
				createBuilderEdge({
					id: `edge-${sourceNodeId}-${newProc.id}-${Date.now()}`,
					source: sourceNodeId,
					target: newProc.id,
					data: isFromSource
						? {
								label: edgeLabel,
								selectedItemIds: edgeItemIds,
								showAddButton: false
							}
						: { showAddButton: false }
				})
			)

			if (isFromSource && newProc.data) {
				newProc.data.itemCount = edgeItemIds.length
			}

			// 4. Hubungkan newProc ke target
			edges.value.push(
				createBuilderEdge({
					id: `edge-${newProc.id}-${targetNodeId}-${Date.now() + 1}`,
					source: newProc.id,
					target: targetNodeId,
					data: { showAddButton: true }
				})
			)

			markDirty()
			return newProc
		}

		function removeNode(id: string) {
			nodes.value = nodes.value.filter((n: any) => n.id !== id)
			edges.value = edges.value.filter((e: any) => e.source !== id && e.target !== id)
			if (selectedNode.value && (selectedNode.value as any).id === id) {
				selectedNode.value = undefined
			}
			markDirty()
		}

		function autoLayout(options?: { ranksep?: number; nodesep?: number }) {
			if (nodes.value.length === 0) return

			const g = new dagre.graphlib.Graph()
			g.setDefaultEdgeLabel(() => ({}))

			const ranksep = options?.ranksep ?? 280
			const nodesep = options?.nodesep ?? 120

			g.setGraph({
				rankdir: 'LR',
				ranksep,
				nodesep,
				align: 'UL'
			})

			const nodeDimensions: Record<string, { width: number; height: number }> = {
				source: { width: 220, height: 180 },
				processor: { width: 220, height: 160 },
				sink: { width: 220, height: 150 }
			}

			nodes.value.forEach((node) => {
				const dim = nodeDimensions[node.type || ''] || { width: 220, height: 160 }
				g.setNode(node.id, { width: dim.width, height: dim.height })
			})

			edges.value.forEach((edge) => {
				if (g.hasNode(edge.source) && g.hasNode(edge.target)) {
					g.setEdge(edge.source, edge.target)
				}
			})

			dagre.layout(g)

			let minX = Infinity
			let minY = Infinity

			nodes.value.forEach((node) => {
				const pos = g.node(node.id)
				if (pos) {
					const dim = nodeDimensions[node.type || ''] || { width: 220, height: 160 }
					const topLeftX = Math.round(pos.x - dim.width / 2)
					const topLeftY = Math.round(pos.y - dim.height / 2)
					node.position = { x: topLeftX, y: topLeftY }
					if (topLeftX < minX) minX = topLeftX
					if (topLeftY < minY) minY = topLeftY
				}
			})

			const targetMinX = 120
			const targetMinY = 150
			const offsetX = targetMinX - (minX === Infinity ? 0 : minX)
			const offsetY = targetMinY - (minY === Infinity ? 0 : minY)

			nodes.value.forEach((node) => {
				node.position = {
					x: Math.round(node.position.x + offsetX),
					y: Math.round(node.position.y + offsetY)
				}
			})

			markDirty()
		}

		function syncProcessorCounts(node: any) {
			if (!node || !node.data) return
			const procs: IGroupProcessorItem[] = node.data.processors || []
			node.data.processCount = procs.length
			node.data.activeCount = procs.filter((p) => p.status !== false).length
		}

		function openPreviewDataModal(nodeId?: string, processorId?: string) {
			previewDataModal.value = {
				isOpen: true,
				nodeId: nodeId || selectedNode.value?.id,
				processorId
			}
		}

		function closePreviewDataModal() {
			previewDataModal.value.isOpen = false
		}

		function addProcessorToGroup(nodeId: string, item: IGroupProcessorItem) {
			const node = nodes.value.find((n: any) => n.id === nodeId)
			if (!node) return
			if (!node.data.processors) node.data.processors = []
			node.data.processors.push(item)
			syncProcessorCounts(node)
			markDirty()
		}

		function updateProcessorInGroup(
			nodeId: string,
			processorId: string,
			updated: Partial<IGroupProcessorItem>
		) {
			const node = nodes.value.find((n: any) => n.id === nodeId)
			if (!node || !node.data?.processors) return
			const idx = node.data.processors.findIndex(
				(p: IGroupProcessorItem) => p.processor_id === processorId
			)
			if (idx !== -1) {
				node.data.processors[idx] = {
					...node.data.processors[idx],
					...updated
				}
				syncProcessorCounts(node)
				markDirty()
			}
		}

		function removeProcessorFromGroup(nodeId: string, processorId: string) {
			const node = nodes.value.find((n: any) => n.id === nodeId)
			if (!node || !node.data?.processors) return
			node.data.processors = node.data.processors.filter(
				(p: IGroupProcessorItem) => p.processor_id !== processorId
			)
			syncProcessorCounts(node)
			markDirty()
		}

		function toggleProcessorStatus(nodeId: string, processorId: string, status?: boolean) {
			const node = nodes.value.find((n: any) => n.id === nodeId)
			if (!node || !node.data?.processors) return
			const item = node.data.processors.find(
				(p: IGroupProcessorItem) => p.processor_id === processorId
			)
			if (item) {
				item.status = status !== undefined ? status : !item.status
				syncProcessorCounts(node)
				markDirty()
			}
		}

		function reorderProcessorGroup(nodeId: string, fromIndex: number, toIndex: number) {
			const node = nodes.value.find((n: any) => n.id === nodeId)
			if (!node || !node.data?.processors) return
			const procs = [...node.data.processors]
			const [moved] = procs.splice(fromIndex, 1)
			procs.splice(toIndex, 0, moved)
			node.data.processors = procs
			markDirty()
		}

		function bulkDisableProcessors(nodeId: string, processorIds: string[]) {
			const node = nodes.value.find((n: any) => n.id === nodeId)
			if (!node || !node.data?.processors) return
			node.data.processors.forEach((p: IGroupProcessorItem) => {
				if (processorIds.includes(p.processor_id)) {
					p.status = false
				}
			})
			syncProcessorCounts(node)
			markDirty()
		}

		function bulkDeleteProcessors(nodeId: string, processorIds: string[]) {
			const node = nodes.value.find((n: any) => n.id === nodeId)
			if (!node || !node.data?.processors) return
			node.data.processors = node.data.processors.filter(
				(p: IGroupProcessorItem) => !processorIds.includes(p.processor_id)
			)
			syncProcessorCounts(node)
			markDirty()
		}

		async function fetchFullProcessorForGroup(processorId: string): Promise<IProcessor | null> {
			try {
				const cUrls = clusterUrl.value ? [clusterUrl.value] : undefined
				const res = await processorsApi.get({ limit: 100 }, cUrls)
				const list = (res?.data as any)?.processors || (res?.data as any)?.pipelines
				if (res?.success && list) {
					const found = list.find(
						(p: any) => (p.processor_id || p.pipeline_id) === processorId
					)
					return found || null
				}
			} catch (err) {
				console.error('Failed to fetch processor details:', err)
			}
			return null
		}

		function openCreateProcessorFormModal(targetNodeId: string) {
			if (!targetNodeId) return
			const processorStore = useProcessorStore()
			const currentCluster = clusterUrl.value || appHelper.getDefaultClusterValue()
			processorStore.toggleProcessorModalForm(
				true,
				FormMode.ADD,
				null,
				async (formData: any) => {
					const newId = generateUuidV7()
					const procType = formData.config?.type ?? formData.config?.module ?? 'null'
					const newItem: IGroupProcessorItem = {
						processor_id: newId,
						name: formData.name || 'Processor',
						module: procType,
						status:
							formData.adm_status !== undefined ? formData.adm_status === 1 : true,
						type: 'Transform',
						config: {
							type: procType,
							module: procType,
							...(procType !== 'null' && formData.config?.config !== undefined
								? { config: formData.config.config }
								: {})
						}
					}
					addProcessorToGroup(targetNodeId, newItem)
					return true
				},
				{
					lockedCluster: currentCluster,
					hideInputItemsAndProcessors: true
				}
			)
		}

		async function openEditProcessorFormModal(targetNodeId: string, item: IGroupProcessorItem) {
			const processorStore = useProcessorStore()
			const p = !item.config ? await fetchFullProcessorForGroup(item.processor_id) : null
			const rawConfig = item.config?.config ?? item.config
			const targetProcessor =
				p ||
				({
					processor_id: item.processor_id,
					name: item.name,
					adm_status: item.status !== false ? 1 : 2,
					config: {
						type: (item.module || 'null') as any,
						module: (item.module || 'null') as any,
						config: rawConfig
					}
				} as IProcessor)

			const currentCluster = clusterUrl.value || appHelper.getDefaultClusterValue()
			processorStore.toggleProcessorModalForm(
				true,
				FormMode.UPDATE,
				targetProcessor,
				async (formData: any) => {
					const procType = formData.config?.type ?? formData.config?.module ?? 'null'
					updateProcessorInGroup(targetNodeId, item.processor_id, {
						name: formData.name,
						module: procType,
						status:
							formData.adm_status !== undefined ? formData.adm_status === 1 : true,
						config: {
							type: procType,
							module: procType,
							...(procType !== 'null' && formData.config?.config !== undefined
								? { config: formData.config.config }
								: {})
						}
					})
					return true
				},
				{
					lockedCluster: currentCluster,
					hideInputItemsAndProcessors: true
				}
			)
		}

		async function saveNodeContent(node: INode, clusterURLs?: string[]): Promise<void> {
			const pId = pipelineId.value
			const cId = canvasId.value
			const nodeId = (node as any).node_id || node.id

			// Fetch fresh node state from backend to retrieve the current last_updated timestamp and existing items
			const nodeRes = await pipelineManagementApi.getPipelineNode(
				{
					pipeline_management_id: pId,
					canvas_id: cId,
					node_id: nodeId,
					query: { limit: 100 }
				},
				clusterURLs
			)

			if (!nodeRes?.success || !nodeRes.data) {
				throw new Error(`Failed to read node before saving: ${node.label || nodeId}`)
			}

			const lastUpdated =
				nodeRes.data.last_updated || (nodeRes.data as any)?.data?.last_updated
			let updatePayload: UpdatePipelineManagementNodeRequest

			if (node.type === 'source') {
				const backendItems: any[] =
					(nodeRes.data as any)?.data?.items ||
					(nodeRes.data as any)?.content?.data?.items ||
					(nodeRes.data as any)?.items ||
					[]
				const existingBackendIds = backendItems
					.map((i: any) =>
						String(typeof i === 'string' ? i : i.item_id || i.id || '').trim()
					)
					.filter(Boolean)
				const canvasItems: any[] = node.data?.items || []
				const canvasItemIds = canvasItems
					.map((i: any) =>
						String(typeof i === 'string' ? i : i.item_id || i.id || '').trim()
					)
					.filter(Boolean)
				const targetIds = Array.isArray(node.data?.items)
					? canvasItemIds
					: getSourceActiveItemIds(nodeId)

				const lowerTargetIds = new Set(targetIds.map((id) => id.toLowerCase()))
				const lowerExistingIds = new Set(existingBackendIds.map((id) => id.toLowerCase()))

				const add = targetIds.filter((id) => !lowerExistingIds.has(id.toLowerCase()))
				const remove = existingBackendIds.filter(
					(id) => !lowerTargetIds.has(id.toLowerCase())
				)

				updatePayload = {
					last_updated: lastUpdated,
					kind: 'source',
					data: {
						...(add.length > 0 ? { add } : {}),
						...(remove.length > 0 ? { remove } : {})
					}
				} as any
			} else if (node.type === 'sink') {
				const sinkId = node.data?.sinkId || null
				updatePayload = {
					last_updated: lastUpdated,
					kind: 'sink',
					data: sinkId ? { sink_id: sinkId } : null
				} as any
			} else {
				// Processor Group
				const backendProcessors: any[] =
					(nodeRes.data as any)?.content?.data?.processors ||
					(nodeRes.data as any)?.data?.processors ||
					[]
				const currentBackendIds = new Set(backendProcessors.map((p: any) => p.processor_id))

				let clusterProcIds = new Set<string>()
				try {
					const clusterProcRes = await processorsApi.get({ limit: 100 }, clusterURLs)
					const clusterList =
						(clusterProcRes?.data as any)?.processors ||
						(clusterProcRes?.data as any)?.pipelines ||
						[]
					clusterProcIds = new Set(
						clusterList.map((cp: any) => cp.processor_id || cp.pipeline_id)
					)
				} catch {
					// Ignore if processorsApi fails
				}

				const rawProcessors = node.data?.processors || []
				const mappedProcessors = rawProcessors.map((p: any) => {
					let cfg = p.config
					if (!cfg || !cfg.type) {
						cfg = {
							type: cfg?.module || p.module || 'null',
							...(cfg?.config !== undefined ? { config: cfg.config } : {})
						}
					}

					let procId = p.processor_id
					// If the processor is new to this group (not in currentBackendIds)
					// and its ID already belongs to a registered cluster processor or missing, assign a fresh UUID
					if (
						!currentBackendIds.has(procId) &&
						(!procId || clusterProcIds.has(procId) || p.source_processor_id)
					) {
						procId = generateUuidV7()
						p.processor_id = procId
					}

					return {
						processor_id: procId,
						name: p.name || 'Processor',
						adm_status:
							p.status !== false
								? 1
								: p.adm_status !== undefined && p.adm_status !== 1
									? p.adm_status
									: 0,
						config: cfg
					}
				})

				updatePayload = {
					last_updated: lastUpdated,
					kind: 'processor_group',
					data: {
						processors: mappedProcessors
					}
				} as any
			}

			const updateRes = await pipelineManagementApi.updatePipelineNode(
				{
					pipeline_management_id: pId,
					canvas_id: cId,
					node_id: nodeId
				},
				updatePayload,
				clusterURLs
			)

			if (!updateRes?.success) {
				// Automatic fallback if backend reports "Processor IDs already used:"
				const errMsg =
					(updateRes as any)?.error?.detail ||
					(updateRes as any)?.error?.title ||
					(updateRes as any)?.message ||
					''
				if (errMsg.includes('Processor IDs already used') && node.type === 'processor') {
					const collidedRaw = errMsg.split('Processor IDs already used:')[1] || ''
					const collidedIds = new Set(collidedRaw.split(',').map((s: string) => s.trim()))
					const rawProcessors = node.data?.processors || []
					const refreshedPayload = JSON.parse(JSON.stringify(updatePayload))
					refreshedPayload.data.processors = (refreshedPayload.data.processors || []).map(
						(p: any, idx: number) => {
							if (collidedIds.has(p.processor_id)) {
								const freshId = generateUuidV7()
								if (rawProcessors[idx]) {
									rawProcessors[idx].processor_id = freshId
								}
								return { ...p, processor_id: freshId }
							}
							return p
						}
					)
					const retryRes = await pipelineManagementApi.updatePipelineNode(
						{
							pipeline_management_id: pId,
							canvas_id: cId,
							node_id: nodeId
						},
						refreshedPayload,
						clusterURLs
					)
					if (retryRes?.success) {
						return
					}
				}
				throw new Error(`Failed to save content for node: ${node.label || nodeId}`)
			}
		}

		async function saveGraph(): Promise<boolean> {
			if (!pipelineId.value || !canvasId.value || canvasId.value === 'default') {
				error.value = 'Canvas ID is invalid.'
				return false
			}

			loading.saving = true
			error.value = null
			successMessage.value = null

			try {
				const cUrls = clusterUrl.value ? [clusterUrl.value] : undefined

				// Map Vue Flow nodes back to Backend GraphNode format
				const backendNodes = nodes.value.map((n: any) => {
					let kind: 'source' | 'processor_group' | 'sink' = 'processor_group'
					let data: any = {}

					if (n.type === 'source') {
						kind = 'source'
						const count =
							typeof n.data?.item_count === 'number'
								? n.data.item_count
								: (n.data?.items?.length ?? n.data?.itemCount ?? 0)
						data = {
							item_count: count
						}
					} else if (n.type === 'sink') {
						kind = 'sink'
						data = n.data?.sinkId ? { sink_id: n.data.sinkId } : null
					} else {
						kind = 'processor_group'
						const rawProcessors = n.data?.processors || []
						const mappedProcessors = rawProcessors.map((p: any) => ({
							processor_id: p.processor_id,
							name: p.name,
							module: p.module || 'null'
						}))
						data = {
							name: n.data?.name || n.label || 'Processor Group',
							processors: mappedProcessors
						}
					}

					const nodeId = (n as any).node_id || n.id

					return {
						node_id: nodeId,
						kind,
						data,
						position_x: Math.round(n.position.x),
						position_y: Math.round(n.position.y)
					}
				})

				const backendRelations = edges.value
					.filter((e: any) => {
						const fromNode = nodes.value.find((n: any) => n.id === e.source)
						const toNode = nodes.value.find((n: any) => n.id === e.target)
						return Boolean(fromNode && toNode)
					})
					.map((e: any) => {
						const fromNode = nodes.value.find((n: any) => n.id === e.source)
						const toNode = nodes.value.find((n: any) => n.id === e.target)
						const rel: any = {
							from_node_id: (fromNode as any)?.node_id || e.source,
							to_node_id: (toNode as any)?.node_id || e.target
						}
						if (e.data?.label) {
							rel.label = e.data.label
						}
						return rel
					})

				const payload: UpdatePipelineManagementGraphRequest = {
					graph: {
						nodes: backendNodes as any,
						relations: backendRelations
					},
					last_updated: canvasGraph.value?.last_updated || new Date().toISOString()
				}

				// Stage 1: Save graph structure
				const res = await pipelineManagementApi.updatePipelineGraph(
					{
						pipeline_management_id: pipelineId.value,
						canvas_id: canvasId.value
					},
					payload,
					cUrls
				)

				if (!res?.success || !res.data) {
					return false
				}

				canvasGraph.value = res.data

				// Stage 2: Save content for each active node in parallel using Promise.allSettled
				const activeNodes = [...nodes.value]

				const nodeResults = await Promise.allSettled(
					activeNodes.map((n) => saveNodeContent(n, cUrls))
				)

				const failures: string[] = []
				nodeResults.forEach((outcome, idx) => {
					if (outcome.status === 'rejected') {
						const nodeName = activeNodes[idx].label || activeNodes[idx].id
						failures.push(`${nodeName} (${outcome.reason?.message || 'unknown error'})`)
					}
				})

				if (failures.length > 0) {
					isDirty.value = true
					error.value = `Graph saved, but failed to update content for node(s): ${failures.join('; ')}`
					return false
				}

				isDirty.value = false
				successMessage.value = 'Pipeline graph and all node contents saved successfully!'
				return true
			} catch (err: any) {
				error.value = err.message || 'Failed to save pipeline graph'
				return false
			} finally {
				loading.saving = false
			}
		}

		return {
			canvasKey,
			pipelineId,
			canvasId,
			clusterUrl,
			pipelineDetail,
			canvasGraph,
			nodes,
			edges,
			selectedNode,
			isCanvasEmpty,
			isDirty,
			loading,
			error,
			successMessage,
			availableHosts,
			availableSinks,
			pipelineItems,
			loadingItems,
			edgeItemModal,
			previewDataModal,
			openPreviewDataModal,
			closePreviewDataModal,
			addProcessorToGroup,
			updateProcessorInGroup,
			removeProcessorFromGroup,
			toggleProcessorStatus,
			reorderProcessorGroup,
			bulkDisableProcessors,
			bulkDeleteProcessors,
			fetchFullProcessorForGroup,
			openCreateProcessorFormModal,
			openEditProcessorFormModal,
			loadBuilder,
			markDirty,
			fetchPipelineItems,
			addSourceNode,
			addProcessorNode,
			addSinkNode,
			selectSinkForNode,
			openEdgeItemSelector,
			closeEdgeItemSelector,
			applyEdgeItems,
			connectNodes,
			insertProcessorBetween,
			removeNode,
			autoLayout,
			saveGraph,
			saveNodeContent,
			getSourceActiveItemIds,
			buildGraphRepresentation,
			// Legacy compatibility
			itemPagination,
			itemFilter,
			items,
			itemLoading,
			itemError,
			fetchItems,
			fetchGraph,
			updateCanvas
		}
	},
	{ persist: false }
)

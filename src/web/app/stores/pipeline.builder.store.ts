// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { defineStore } from 'pinia'
import { computed, reactive, ref } from 'vue'
import axios from 'axios'
import pipelinesApi from '@/app/api/pipelines.api'
import hostsApi from '@/app/api/hosts.api'
import sinksApi from '@/app/api/sinks.api'
import itemsApi from '@/app/api/items.api'
import processorsApi from '@/app/api/processors.api'
import appHelper from '@/app/helpers/app.helper'
import { generateUuidV7 } from '@/app/helpers/uuid.helper'
import { useAppStore } from '@/stores/app.store'
import { useProcessorStore } from '@/stores/processor.store'
import { FormMode } from '@@/shared/types/app.type'
import type { IProcessor } from '@@/shared/types/processor.type'
import { MarkerType } from '@vue-flow/core'
import dagre from '@dagrejs/dagre'
import type { INode } from '@@/shared/types/canvas.type'
import type {
	GetPipelineGraphResponse,
	PipelineDetail,
	UpdatePipelineGraphRequest,
	UpdatePipelineNodeRequest,
	UpdatePipelineRequest
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

		const pipelineDetail = ref<PipelineDetail | null>(null)
		const canvasGraph = ref<GetPipelineGraphResponse | null>(null)

		const nodes = ref<any[]>([])
		const edges = ref<any[]>([])
		const selectedNode = ref<any>()
		const isCanvasEmpty = computed(() => nodes.value.length === 0)
		const hasSourceNode = computed(() => nodes.value.some((n: any) => n.type === 'source'))

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

		const hostById = computed(() => {
			const map = new Map<string, any>()
			for (const h of availableHosts.value) {
				if (h?.host_id) map.set(h.host_id, h)
			}
			return map
		})

		const sinkById = computed(() => {
			const map = new Map<string, any>()
			for (const s of availableSinks.value) {
				if (s?.sink_id) map.set(s.sink_id, s)
			}
			return map
		})

		function resolveHostNames(hostIds: string[]): string[] {
			const lookup = hostById.value
			return hostIds.map((hid) => lookup.get(hid)?.name || hid)
		}

		const hostSelectorModalOpen = ref(false)
		const sinkSelectorModalOpen = ref(false)
		const selectorError = ref<string | null>(null)
		const loadingResources = reactive({
			hosts: false,
			sinks: false,
			updatingSubset: false
		})
		const subsetConfirmation = ref<{
			isOpen: boolean
			kind: 'hosts' | 'sinks'
			nextIds: string[]
			message: string
		}>({
			isOpen: false,
			kind: 'hosts',
			nextIds: [],
			message: ''
		})

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
				pipelineItems.value = await fetchAllPages(async (limit, offset) => {
					const res = await itemsApi.getItems(
						offset > 0
							? { host_ids: hostIds, limit, offset }
							: { host_ids: hostIds, limit },
						cUrls
					)
					return {
						items: res?.data?.items || [],
						total: res?.data?.total
					}
				})
			} catch (err) {
				console.error('Failed to fetch pipeline items:', err)
			} finally {
				loadingItems.value = false
			}
		}

		function markDirty() {
			isDirty.value = true
		}

		const RESOURCE_PAGE_LIMIT = 100
		const MAX_RESOURCE_PAGES = 50

		async function fetchAllPages<T>(
			fetchPage: (limit: number, offset: number) => Promise<{ items: T[]; total?: number }>
		): Promise<T[]> {
			const firstPage = await fetchPage(RESOURCE_PAGE_LIMIT, 0)
			const collected: T[] = [...firstPage.items]
			if (
				firstPage.items.length === 0 ||
				firstPage.items.length < RESOURCE_PAGE_LIMIT ||
				(typeof firstPage.total === 'number' && collected.length >= firstPage.total)
			) {
				return collected
			}

			if (typeof firstPage.total === 'number' && firstPage.total > collected.length) {
				const totalPages = Math.min(
					MAX_RESOURCE_PAGES,
					Math.ceil(firstPage.total / RESOURCE_PAGE_LIMIT)
				)
				const pagePromises: Promise<{ items: T[]; total?: number }>[] = []
				for (let page = 1; page < totalPages; page++) {
					pagePromises.push(fetchPage(RESOURCE_PAGE_LIMIT, page * RESOURCE_PAGE_LIMIT))
				}
				const remainingPages = await Promise.all(pagePromises)
				for (const pageRes of remainingPages) {
					collected.push(...pageRes.items)
				}
				return collected
			}

			for (let page = 1; page < MAX_RESOURCE_PAGES; page++) {
				const offset = page * RESOURCE_PAGE_LIMIT
				const { items, total } = await fetchPage(RESOURCE_PAGE_LIMIT, offset)
				collected.push(...items)
				if (
					items.length === 0 ||
					items.length < RESOURCE_PAGE_LIMIT ||
					(typeof total === 'number' && collected.length >= total)
				) {
					break
				}
			}
			return collected
		}

		async function fetchAllHosts(clusterURLs?: string[]): Promise<any[]> {
			return fetchAllPages(async (limit, offset) => {
				const res = await hostsApi.getHosts(
					offset > 0 ? { limit, offset } : { limit },
					clusterURLs
				)
				return {
					items: res?.data?.hosts || [],
					total: res?.data?.total
				}
			})
		}

		async function fetchAllSinks(clusterURLs?: string[]): Promise<any[]> {
			return fetchAllPages(async (limit, offset) => {
				const res = await sinksApi.getSinks(
					offset > 0 ? { limit, offset } : { limit },
					clusterURLs
				)
				return {
					items: res?.data?.sinks || [],
					total: res?.data?.total
				}
			})
		}

		async function loadBuilder(pId: string, cId: string, cluster?: string) {
			pipelineId.value = pId
			canvasId.value = cId
			clusterUrl.value = cluster || null
			loading.canvas = true
			error.value = null
			isDirty.value = false
			selectedNode.value = undefined
			hostSelectorModalOpen.value = false
			sinkSelectorModalOpen.value = false
			selectorError.value = null
			subsetConfirmation.value.isOpen = false

			const cUrls = clusterUrl.value ? [clusterUrl.value] : undefined

			try {
				const shouldFetchGraph = Boolean(cId && cId !== 'default')

				const [detailRes, hostsList, sinksList, graphRes] = await Promise.all([
					pipelinesApi.getPipelineById(pId, cUrls),
					fetchAllHosts(cUrls).catch(() => [] as any[]),
					fetchAllSinks(cUrls).catch(() => [] as any[]),
					shouldFetchGraph
						? pipelinesApi.getPipelineGraph({ pipeline_id: pId, canvas_id: cId }, cUrls)
						: Promise.resolve(undefined)
				])

				availableHosts.value = hostsList
				availableSinks.value = sinksList

				const nodeTasks: Promise<unknown>[] = []

				if (detailRes?.success && detailRes.data) {
					pipelineDetail.value = detailRes.data
					nodeTasks.push(fetchPipelineItems())
				}

				if (graphRes?.success && graphRes.data) {
					canvasGraph.value = graphRes.data
					const graphNodes = graphRes.data.graph?.nodes || []

					const sourceNode = graphNodes.find((n: any) => n.kind === 'source')
					if (sourceNode?.node_id) {
						nodeTasks.push(
							fetchSourceNodeDetails(pId, cId, sourceNode.node_id, cUrls)
								.then((sourceDetails) => {
									if (!sourceDetails) return
									const { items: sourceItems, total } = sourceDetails
									if (!sourceNode.data) (sourceNode as any).data = {}
									;(sourceNode.data as any).items = sourceItems
									if (typeof total === 'number') {
										;(sourceNode.data as any).item_count = total
									} else if (sourceItems.length > 0) {
										;(sourceNode.data as any).item_count = sourceItems.length
									}
								})
								.catch((e) => {
									console.error(
										'Failed to fetch source node items in loadCanvas:',
										e
									)
								})
						)
					}

					const procNodes = graphNodes.filter((n: any) => n.kind === 'processor_group')
					for (const pn of procNodes) {
						if (!pn?.node_id) continue
						nodeTasks.push(
							pipelinesApi
								.getPipelineNode(
									{
										pipeline_id: pId,
										canvas_id: cId,
										node_id: pn.node_id
									},
									cUrls
								)
								.then((pnRes) => {
									const procList =
										(pnRes?.data as any)?.data?.processors ||
										(pnRes?.data as any)?.content?.data?.processors ||
										(pnRes?.data as any)?.processors
									if (procList && Array.isArray(procList)) {
										if (!pn.data) pn.data = {}
										;(pn.data as any).processors = procList
									}
								})
								.catch((e) => {
									console.error(
										'Failed to fetch processor group node content in loadCanvas:',
										e
									)
								})
						)
					}
				}

				if (nodeTasks.length > 0) {
					await Promise.allSettled(nodeTasks)
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
						const hostNames = resolveHostNames(pipelineDetail.value?.host_ids || [])
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
						const sinkMeta = sinkId ? sinkById.value.get(sinkId) : undefined
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
				const existingNodeById = new Map<string, any>(
					existingNodes.map((gn: any) => [gn.node_id, gn])
				)

				const mappedEdges: any[] = existingRelations.map((rel: any, idx) => {
					const fromNodeMeta: any = existingNodeById.get(rel.from_node_id)
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
				syncDownstreamItemCounts()

				return
			}

			// Empty State: when api graph.nodes response is empty, canvas enters empty state
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

		function addSourceNode(x?: number, y?: number): INode | null {
			if (hasSourceNode.value) {
				const message = 'A pipeline can only have one Source node.'
				error.value = message
				notifyNonAxiosError('Source Node Limit', message)
				return null
			}

			const nodeId = generateUuidV7()
			const posX = x ?? 80
			const posY = y ?? 160

			const hosts = pipelineDetail.value?.host_ids || []
			const hostNames = resolveHostNames(hosts)

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
			const targetSink = sinkById.value.get(sinkId)
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

		function parseEdgeLabelCount(label?: string): number | null {
			if (typeof label !== 'string') return null
			const match = label.match(/^(\d+)\s+Items?$/i)
			return match ? Number(match[1]) : null
		}

		function getSourceNodeItemCount(sourceNode: any): number {
			if (!sourceNode?.data) return 0
			if (typeof sourceNode.data.item_count === 'number') return sourceNode.data.item_count
			if (Array.isArray(sourceNode.data.items) && sourceNode.data.items.length > 0) {
				return sourceNode.data.items.length
			}
			return typeof sourceNode.data.itemCount === 'number' ? sourceNode.data.itemCount : 0
		}

		/** Propagate incoming item counts from Source nodes to all downstream processor nodes. */
		function syncDownstreamItemCounts() {
			const nodeById = new Map<string, any>()
			const sourceNodes: any[] = []
			const processorNodes: any[] = []

			for (const node of nodes.value as any[]) {
				nodeById.set(node.id, node)
				if (node.type === 'source') sourceNodes.push(node)
				else if (node.type === 'processor' && node.data) processorNodes.push(node)
			}

			const outgoingEdges = new Map<string, any[]>()
			for (const edge of edges.value as any[]) {
				const list = outgoingEdges.get(edge.source)
				if (list) list.push(edge)
				else outgoingEdges.set(edge.source, [edge])
			}

			const incomingCount = new Map<string, number>()
			const queue: Array<{ nodeId: string; count: number }> = []

			const enqueueTarget = (targetId: string, count: number) => {
				const target = nodeById.get(targetId)
				if (!target || target.type !== 'processor') return
				const prev = incomingCount.get(target.id) ?? -1
				if (count > prev) {
					incomingCount.set(target.id, count)
					queue.push({ nodeId: target.id, count })
				}
			}

			for (const node of sourceNodes) {
				const fallbackSourceCount = getSourceNodeItemCount(node)
				for (const edge of outgoingEdges.get(node.id) || []) {
					const selectedIds = edge.data?.selectedItemIds
					const labelCount = parseEdgeLabelCount(edge.data?.label)
					const edgeCount =
						Array.isArray(selectedIds) && selectedIds.length > 0
							? fallbackSourceCount > selectedIds.length &&
								labelCount === fallbackSourceCount
								? fallbackSourceCount
								: selectedIds.length
							: (labelCount ?? fallbackSourceCount)
					enqueueTarget(edge.target, edgeCount)
				}
			}

			let head = 0
			while (head < queue.length) {
				const current = queue[head++]!
				if (current.count < (incomingCount.get(current.nodeId) ?? 0)) continue

				for (const edge of outgoingEdges.get(current.nodeId) || []) {
					enqueueTarget(edge.target, current.count)
				}
			}

			for (const node of processorNodes) {
				const nextCount = incomingCount.get(node.id) ?? 0
				if (node.data.itemCount !== nextCount) {
					node.data.itemCount = nextCount
				}
			}
		}

		function applyEdgeItems(edgeId: string, itemIds: string[]) {
			const edge = edges.value.find((e: any) => e.id === edgeId)
			if (edge) {
				if (!edge.data) edge.data = {}
				edge.data.selectedItemIds = itemIds
				edge.data.label = `${itemIds.length} Items`
				edge.data.showAddButton = false

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
							}
						}
					})

					// Trigger PUT /api/pipelines/{id}/canvases/{cId}/nodes/{nodeId} only if node already exists in backend graph
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

				syncDownstreamItemCounts()
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
				syncDownstreamItemCounts()
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

			// 1. Create new processor node
			const newProc = addProcessorNode(x, y)

			// 2. Remove old cut edge so it doesn't cross through node
			edges.value.splice(edgeIndex, 1)

			// 3. Connect source to newProc
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

			// 4. Connect newProc to target
			edges.value.push(
				createBuilderEdge({
					id: `edge-${newProc.id}-${targetNodeId}-${Date.now() + 1}`,
					source: newProc.id,
					target: targetNodeId,
					data: { showAddButton: true }
				})
			)

			syncDownstreamItemCounts()
			markDirty()
			return newProc
		}

		function removeNode(id: string) {
			nodes.value = nodes.value.filter((n: any) => n.id !== id)
			edges.value = edges.value.filter((e: any) => e.source !== id && e.target !== id)
			if (selectedNode.value && (selectedNode.value as any).id === id) {
				selectedNode.value = undefined
			}
			syncDownstreamItemCounts()
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
			const targetIds = new Set(processorIds)
			node.data.processors.forEach((p: IGroupProcessorItem) => {
				if (targetIds.has(p.processor_id)) {
					p.status = false
				}
			})
			syncProcessorCounts(node)
			markDirty()
		}

		function bulkDeleteProcessors(nodeId: string, processorIds: string[]) {
			const node = nodes.value.find((n: any) => n.id === nodeId)
			if (!node || !node.data?.processors) return
			const targetIds = new Set(processorIds)
			node.data.processors = node.data.processors.filter(
				(p: IGroupProcessorItem) => !targetIds.has(p.processor_id)
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

		function buildProcessorPayloadConfig(formData: any) {
			const procType = formData.config?.type ?? formData.config?.module ?? 'null'
			return {
				procType,
				config: {
					type: procType,
					module: procType,
					...(procType !== 'null' && formData.config?.config !== undefined
						? { config: formData.config.config }
						: {})
				}
			}
		}

		/** Mutate a processor group and persist per-node via saveNodeContent() (or executeSaveGraph() for unsaved nodes), rolling back on failure. */
		async function commitProcessorChange(
			targetNodeId: string,
			mutate: () => void
		): Promise<boolean> {
			if (loading.saving) return false

			const node = nodes.value.find((n: any) => n.id === targetNodeId)
			if (!node) return false

			const previousProcessors = node.data?.processors
				? JSON.parse(JSON.stringify(node.data.processors))
				: []
			const wasDirty = isDirty.value

			const rollbackTarget = () => {
				if (node.data) {
					node.data.processors = previousProcessors
					syncProcessorCounts(node)
				}
			}

			const existsInBackend = Boolean(
				canvasGraph.value?.graph?.nodes?.some(
					(gn: any) => gn.node_id === ((node as any).node_id || node.id)
				)
			)

			if (
				existsInBackend &&
				pipelineId.value &&
				canvasId.value &&
				canvasId.value !== 'default'
			) {
				mutate()
				loading.saving = true
				error.value = null
				try {
					const cUrls = clusterUrl.value ? [clusterUrl.value] : undefined
					await saveNodeContent(node, cUrls)
					if (!wasDirty) {
						isDirty.value = false
					}
					return true
				} catch (err: any) {
					rollbackTarget()
					isDirty.value = wasDirty
					error.value = extractErrorMessage(err, 'Failed to save processor node')
					if (!isHttpOrAxiosError(err)) {
						notifyNonAxiosError('Failed to save processor', error.value ?? '')
					}
					return false
				} finally {
					loading.saving = false
				}
			}

			mutate()
			const result = await executeSaveGraph()
			const targetFailed = !result.stage1Ok || result.failedNodeIds.has(targetNodeId)

			if (targetFailed) {
				rollbackTarget()
				if (!wasDirty && (!result.stage1Ok || result.failedNodeIds.size <= 1)) {
					isDirty.value = false
				}
				return false
			}

			return true
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
					const { procType, config } = buildProcessorPayloadConfig(formData)
					const newItem: IGroupProcessorItem = {
						processor_id: newId,
						name: formData.name || 'Processor',
						module: procType,
						status:
							formData.adm_status !== undefined ? formData.adm_status === 1 : true,
						type: 'Transform',
						config
					}
					return commitProcessorChange(targetNodeId, () => {
						addProcessorToGroup(targetNodeId, newItem)
					})
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
					const { procType, config } = buildProcessorPayloadConfig(formData)
					return commitProcessorChange(targetNodeId, () => {
						updateProcessorInGroup(targetNodeId, item.processor_id, {
							name: formData.name,
							module: procType,
							status:
								formData.adm_status !== undefined
									? formData.adm_status === 1
									: true,
							config
						})
					})
				},
				{
					lockedCluster: currentCluster,
					hideInputItemsAndProcessors: true
				}
			)
		}

		async function fetchClusterProcessorIds(clusterURLs?: string[]): Promise<Set<string>> {
			try {
				const clusterList = await fetchAllPages(async (limit, offset) => {
					const clusterProcRes = await processorsApi.get(
						offset > 0 ? { limit, offset } : { limit },
						clusterURLs
					)
					const items =
						(clusterProcRes?.data as any)?.processors ||
						(clusterProcRes?.data as any)?.pipelines ||
						[]
					return {
						items,
						total: (clusterProcRes?.data as any)?.total
					}
				})
				return new Set<string>(
					clusterList.map((cp: any) => cp.processor_id || cp.pipeline_id)
				)
			} catch {
				return new Set<string>()
			}
		}

		function parseSourceNodeContent(sourceNodeRes: any): { items: any[]; total?: number } {
			const contentData =
				sourceNodeRes?.data?.content?.data ??
				sourceNodeRes?.data?.data ??
				sourceNodeRes?.data
			const items = Array.isArray(contentData?.items) ? contentData.items : []
			const total = typeof contentData?.total === 'number' ? contentData.total : undefined
			return { items, total }
		}

		async function fetchSourceNodeDetails(
			pId: string,
			cId: string,
			nodeId: string,
			clusterURLs?: string[]
		): Promise<{ response: any; items: any[]; total?: number } | null> {
			let firstRes: any = null
			let firstTotal: number | undefined

			const items = await fetchAllPages(async (limit, offset) => {
				const res = await pipelinesApi.getPipelineNode(
					{
						pipeline_id: pId,
						canvas_id: cId,
						node_id: nodeId,
						query: offset > 0 ? { limit, offset } : { limit }
					},
					clusterURLs
				)
				if (offset === 0) {
					firstRes = res
				}
				if (!res?.success || !res.data) {
					return { items: [], total: offset === 0 ? 0 : undefined }
				}
				const parsed = parseSourceNodeContent(res)
				if (offset === 0) {
					firstTotal = parsed.total
				}
				return parsed
			})

			if (!firstRes?.success || !firstRes.data) {
				return null
			}

			return {
				response: firstRes,
				items,
				total: firstTotal
			}
		}

		function isHttpOrAxiosError(err: unknown): boolean {
			if (axios.isAxiosError(err)) return true
			if (!err || typeof err !== 'object') return false
			return (
				('status' in err && (err as { status?: unknown }).status !== undefined) ||
				('data' in err && (err as { data?: unknown }).data !== undefined)
			)
		}

		function extractErrorMessage(err: any, fallback: string): string {
			return err?.data?.error?.detail || err?.data?.error?.title || err?.message || fallback
		}

		function notifyNonAxiosError(title: string, message: string) {
			try {
				useAppStore().showError(title, message)
			} catch {
				// Ignore when Pinia app store is unavailable
			}
		}

		function extractRawItemId(item: any): string {
			return String(typeof item === 'string' ? item : item?.item_id || item?.id || '').trim()
		}

		function normalizeItemId(item: any): string {
			return extractRawItemId(item).toLowerCase()
		}

		function extractTrimmedItemIds(items: any[]): string[] {
			return items.map(extractRawItemId).filter(Boolean)
		}

		async function saveNodeContent(
			node: INode,
			clusterURLs?: string[],
			getClusterProcIds?: () => Promise<Set<string>>
		): Promise<void> {
			const pId = pipelineId.value
			const cId = canvasId.value
			const nodeId = (node as any).node_id || node.id

			// Fetch fresh node state from backend to retrieve the current last_updated timestamp and existing items
			const isSourceNode = node.type === 'source' && Boolean(pId && cId)
			const sourceDetails = isSourceNode
				? await fetchSourceNodeDetails(pId!, cId!, nodeId, clusterURLs)
				: null
			const nodeRes = isSourceNode
				? sourceDetails?.response
				: await pipelinesApi.getPipelineNode(
						{
							pipeline_id: pId,
							canvas_id: cId,
							node_id: nodeId,
							query: { limit: RESOURCE_PAGE_LIMIT }
						},
						clusterURLs
					)

			if (!nodeRes?.success || !nodeRes.data) {
				throw new Error(`Failed to read node before saving: ${node.label || nodeId}`)
			}

			const lastUpdated =
				nodeRes.data.last_updated || (nodeRes.data as any)?.data?.last_updated
			let updatePayload: UpdatePipelineNodeRequest

			if (node.type === 'source') {
				const existingBackendIds = extractTrimmedItemIds(sourceDetails?.items || [])
				const canvasItemIds = extractTrimmedItemIds(node.data?.items || [])
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

				const rawProcessors = node.data?.processors || []
				const needsClusterProcLookup = rawProcessors.some(
					(p: any) =>
						p.processor_id &&
						!currentBackendIds.has(p.processor_id) &&
						!p.source_processor_id
				)
				const clusterProcIds = needsClusterProcLookup
					? await (getClusterProcIds
							? getClusterProcIds()
							: fetchClusterProcessorIds(clusterURLs))
					: new Set<string>()

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

			const updateRes = await pipelinesApi.updatePipelineNode(
				{
					pipeline_id: pId,
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
					const retryRes = await pipelinesApi.updatePipelineNode(
						{
							pipeline_id: pId,
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

		interface SaveGraphOutcome {
			ok: boolean
			stage1Ok: boolean
			failedNodeIds: Set<string>
		}

		async function executeSaveGraph(): Promise<SaveGraphOutcome> {
			if (loading.saving) {
				return { ok: false, stage1Ok: false, failedNodeIds: new Set() }
			}

			if (!pipelineId.value || !canvasId.value || canvasId.value === 'default') {
				error.value = 'Canvas ID is invalid.'
				notifyNonAxiosError('Failed to save pipeline', error.value)
				return { ok: false, stage1Ok: false, failedNodeIds: new Set() }
			}

			const sourceNodeCount = nodes.value.filter((n: any) => n.type === 'source').length
			if (sourceNodeCount > 1) {
				error.value = 'A pipeline can only have one Source node.'
				notifyNonAxiosError('Failed to save pipeline', error.value)
				return { ok: false, stage1Ok: false, failedNodeIds: new Set() }
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

				const nodeById = new Map<string, any>(nodes.value.map((n: any) => [n.id, n]))
				const backendRelations = edges.value.flatMap((e: any) => {
					const fromNode = nodeById.get(e.source)
					const toNode = nodeById.get(e.target)
					if (!fromNode || !toNode) return []
					const rel: any = {
						from_node_id: fromNode.node_id || e.source,
						to_node_id: toNode.node_id || e.target
					}
					if (e.data?.label) {
						rel.label = e.data.label
					}
					return [rel]
				})

				const payload: UpdatePipelineGraphRequest = {
					graph: {
						nodes: backendNodes as any,
						relations: backendRelations
					},
					last_updated: canvasGraph.value?.last_updated || new Date().toISOString()
				}

				// Stage 1: Save graph structure
				const res = await pipelinesApi.updatePipelineGraph(
					{
						pipeline_id: pipelineId.value,
						canvas_id: canvasId.value
					},
					payload,
					cUrls
				)

				if (!res?.success || !res.data) {
					const msg =
						(res as any)?.error?.detail ||
						(res as any)?.error?.title ||
						'Failed to save pipeline graph'
					error.value = msg
					notifyNonAxiosError('Failed to save pipeline', msg)
					return { ok: false, stage1Ok: false, failedNodeIds: new Set() }
				}

				canvasGraph.value = res.data

				// Stage 2: Save content for each active node in parallel using Promise.allSettled
				const activeNodes = [...nodes.value]
				let clusterProcIdsPromise: Promise<Set<string>> | null = null
				const getClusterProcIds = () => {
					if (!clusterProcIdsPromise) {
						clusterProcIdsPromise = fetchClusterProcessorIds(cUrls)
					}
					return clusterProcIdsPromise
				}

				const nodeResults = await Promise.allSettled(
					activeNodes.map((n) => saveNodeContent(n, cUrls, getClusterProcIds))
				)

				const failures: string[] = []
				const failedNodeIds = new Set<string>()
				let hasNonAxiosFailure = false
				nodeResults.forEach((outcome, idx) => {
					if (outcome.status === 'rejected') {
						const failedNode = activeNodes[idx]
						failedNodeIds.add(failedNode.id)
						if (!isHttpOrAxiosError(outcome.reason)) {
							hasNonAxiosFailure = true
						}
						const nodeName = failedNode.label || failedNode.id
						failures.push(
							`${nodeName} (${extractErrorMessage(outcome.reason, 'unknown error')})`
						)
					}
				})

				if (failures.length > 0) {
					isDirty.value = true
					error.value = `Graph saved, but failed to update content for node(s): ${failures.join('; ')}`
					if (hasNonAxiosFailure) {
						notifyNonAxiosError('Failed to save pipeline', error.value)
					}
					return { ok: false, stage1Ok: true, failedNodeIds }
				}

				isDirty.value = false
				successMessage.value = 'Pipeline graph and all node contents saved successfully!'
				return { ok: true, stage1Ok: true, failedNodeIds }
			} catch (err: any) {
				error.value = extractErrorMessage(err, 'Failed to save pipeline graph')
				if (!isHttpOrAxiosError(err)) {
					notifyNonAxiosError('Failed to save pipeline', error.value ?? '')
				}
				return { ok: false, stage1Ok: false, failedNodeIds: new Set() }
			} finally {
				loading.saving = false
			}
		}

		function clearSelectorError() {
			selectorError.value = null
		}

		async function openHostSelectorModal() {
			selectorError.value = null
			hostSelectorModalOpen.value = true
			if (loadingResources.hosts) return
			loadingResources.hosts = true
			const cUrls = clusterUrl.value ? [clusterUrl.value] : undefined
			try {
				availableHosts.value = await fetchAllHosts(cUrls)
			} catch (err) {
				console.error('Failed to refresh available hosts:', err)
				selectorError.value = 'Failed to load available hosts.'
			} finally {
				loadingResources.hosts = false
			}
		}

		function closeHostSelectorModal() {
			if (loadingResources.updatingSubset) return
			hostSelectorModalOpen.value = false
			selectorError.value = null
		}

		async function openSinkSelectorModal() {
			selectorError.value = null
			sinkSelectorModalOpen.value = true
			if (loadingResources.sinks) return
			loadingResources.sinks = true
			const cUrls = clusterUrl.value ? [clusterUrl.value] : undefined
			try {
				availableSinks.value = await fetchAllSinks(cUrls)
			} catch (err) {
				console.error('Failed to refresh available sinks:', err)
				selectorError.value = 'Failed to load available sinks.'
			} finally {
				loadingResources.sinks = false
			}
		}

		function closeSinkSelectorModal() {
			if (loadingResources.updatingSubset) return
			sinkSelectorModalOpen.value = false
			selectorError.value = null
		}

		function cancelSubsetConfirmation() {
			if (loadingResources.updatingSubset) return
			subsetConfirmation.value.isOpen = false
		}

		function formatSubsetInUseMessage(itemCount: number, sinkNodeCount: number): string {
			const parts: string[] = []
			if (itemCount > 0) parts.push(`${itemCount} item(s)`)
			if (sinkNodeCount > 0) parts.push(`${sinkNodeCount} sink node(s)`)
			return `${parts.join(' and ')} on the canvas use hosts or sinks that would be removed. Removing them will also take them off the canvas. Continue?`
		}

		function forEachItemWithHost(cb: (id: string, hostId: string) => void) {
			for (const item of pipelineItems.value as any[]) {
				if (!item?.host_id) continue
				const id = normalizeItemId(item)
				if (id) cb(id, item.host_id)
			}
			for (const node of nodes.value as any[]) {
				if (node.type !== 'source') continue
				for (const item of (node.data?.items || []) as any[]) {
					if (item && typeof item === 'object' && item.host_id) {
						const id = normalizeItemId(item)
						if (id) cb(id, item.host_id)
					}
				}
			}
		}

		function findLocalSubsetUsage(kind: 'hosts' | 'sinks', droppedIds: string[]) {
			const droppedSet = new Set(droppedIds)
			if (droppedSet.size === 0) {
				return { itemCount: 0, sinkNodeCount: 0 }
			}

			if (kind === 'sinks') {
				const sinkNodeCount = nodes.value.filter(
					(n: any) => n.type === 'sink' && n.data?.sinkId && droppedSet.has(n.data.sinkId)
				).length
				return { itemCount: 0, sinkNodeCount }
			}

			const itemHostById = new Map<string, string>()
			forEachItemWithHost((id, hostId) => {
				itemHostById.set(id, hostId)
			})

			const affectedItemIds = new Set<string>()
			for (const node of nodes.value as any[]) {
				if (node.type !== 'source') continue
				const sourceIds = [
					...extractItemIds(node.data?.items || []),
					...getSourceActiveItemIds(node.id)
				]
				for (const itemId of sourceIds) {
					const hostId = itemHostById.get(itemId.toLowerCase())
					if (hostId && droppedSet.has(hostId)) {
						affectedItemIds.add(itemId.toLowerCase())
					}
				}
			}

			return { itemCount: affectedItemIds.size, sinkNodeCount: 0 }
		}

		function collectRemovedItemIds(droppedHostIds: string[]): Set<string> {
			const removedItemIds = new Set<string>()
			if (droppedHostIds.length === 0) return removedItemIds

			const droppedHostSet = new Set(droppedHostIds)
			forEachItemWithHost((id, hostId) => {
				if (droppedHostSet.has(hostId)) {
					removedItemIds.add(id)
				}
			})
			return removedItemIds
		}

		async function refreshSourceNodeFromBackend(node: any): Promise<number | null> {
			const nodeId = (node as any).node_id || node.id
			const existsInBackend = Boolean(
				canvasGraph.value?.graph?.nodes?.some((gn: any) => gn.node_id === nodeId)
			)
			if (
				!existsInBackend ||
				!pipelineId.value ||
				!canvasId.value ||
				canvasId.value === 'default'
			) {
				return null
			}

			try {
				const cUrls = clusterUrl.value ? [clusterUrl.value] : undefined
				const sourceDetails = await fetchSourceNodeDetails(
					pipelineId.value,
					canvasId.value,
					nodeId,
					cUrls
				)
				if (!sourceDetails) return null

				const totalCount = sourceDetails.total ?? sourceDetails.items.length
				node.data.items = sourceDetails.items
				node.data.itemCount = totalCount
				node.data.item_count = totalCount
				return totalCount
			} catch (err) {
				console.error('Failed to refresh source node after forced host removal:', err)
				return null
			}
		}

		function reconcileSourceNodeLocally(node: any, removedItemIds: Set<string>): number {
			const prevCount = getSourceNodeItemCount(node)
			const beforeLen = Array.isArray(node.data.items) ? node.data.items.length : 0
			if (removedItemIds.size > 0 && Array.isArray(node.data.items)) {
				node.data.items = node.data.items.filter(
					(item: any) => !removedItemIds.has(normalizeItemId(item))
				)
			}
			const afterLen = Array.isArray(node.data.items) ? node.data.items.length : 0
			const removedLocally = Math.max(0, beforeLen - afterLen)
			const totalCount = Math.max(0, prevCount - removedLocally)
			node.data.itemCount = totalCount
			node.data.item_count = totalCount
			return totalCount
		}

		function syncSourceOutgoingEdges(
			sourceNode: any,
			totalCount: number,
			refreshedFromBackend: boolean,
			removedItemIds: Set<string>
		) {
			const nodeById = new Map<string, any>(nodes.value.map((n: any) => [n.id, n]))
			const refreshedIds = refreshedFromBackend
				? extractItemIds(sourceNode.data.items || [])
				: []

			edges.value.forEach((e: any) => {
				if (e.source !== sourceNode.id) return
				const targetNode = nodeById.get(e.target)
				if (targetNode?.type !== 'processor') return
				if (!e.data) e.data = {}

				if (refreshedFromBackend) {
					e.data.selectedItemIds = [...refreshedIds]
					e.data.label = `${totalCount} Items`
					return
				}

				const currentEdgeIds: string[] = Array.isArray(e.data.selectedItemIds)
					? e.data.selectedItemIds
					: []
				const nextEdgeIds =
					removedItemIds.size > 0
						? currentEdgeIds.filter((id) => !removedItemIds.has(id.toLowerCase()))
						: currentEdgeIds
				e.data.selectedItemIds = nextEdgeIds
				const displayCount =
					totalCount > nextEdgeIds.length && currentEdgeIds.length === nextEdgeIds.length
						? totalCount
						: nextEdgeIds.length
				e.data.label = `${displayCount} Items`
			})
		}

		function clearDroppedSinkNodes(allowedSinkIds: Set<string>) {
			for (const node of nodes.value as any[]) {
				if (node.type !== 'sink') continue
				if (!node.data?.sinkId || allowedSinkIds.has(node.data.sinkId)) continue

				node.data.sinkId = undefined
				node.data.sinkName = undefined
				node.data.type = 'No sink selected'
				if (selectedNode.value && selectedNode.value.id === node.id) {
					selectedNode.value = { ...node }
				}
			}
		}

		function syncCanvasTimestampFromDetail(detail: PipelineDetail) {
			if (canvasGraph.value && canvasId.value && Array.isArray(detail.canvases)) {
				const matchedCanvas = detail.canvases.find((c) => c.canvas_id === canvasId.value)
				if (matchedCanvas?.last_updated) {
					canvasGraph.value.last_updated = matchedCanvas.last_updated
				}
			}
		}

		async function syncCanvasAfterResourceUpdate(
			updatedDetail: PipelineDetail,
			droppedHostIds: string[],
			droppedSinkIds: string[],
			forced: boolean
		) {
			const previousHostIds = new Set(pipelineDetail.value?.host_ids || [])
			const removedItemIds = forced
				? collectRemovedItemIds(droppedHostIds)
				: new Set<string>()

			pipelineDetail.value = updatedDetail
			syncCanvasTimestampFromDetail(updatedDetail)

			const nextHostIds = updatedDetail.host_ids || []
			const hostsChanged =
				nextHostIds.length !== previousHostIds.size ||
				nextHostIds.some((id) => !previousHostIds.has(id))
			if (hostsChanged) {
				await fetchPipelineItems()
			}

			const hostNames = resolveHostNames(nextHostIds)

			for (const node of nodes.value as any[]) {
				if (node.type !== 'source') continue
				if (!node.data) node.data = {}
				node.data.hostCount = nextHostIds.length
				node.data.hosts = hostNames
				node.data.health = {
					healthy: Math.max(0, nextHostIds.length),
					warning: 0,
					error: 0
				}

				if (!forced || droppedHostIds.length === 0) continue

				const backendTotal = await refreshSourceNodeFromBackend(node)
				const refreshedFromBackend = backendTotal !== null
				const totalCount = refreshedFromBackend
					? backendTotal
					: reconcileSourceNodeLocally(node, removedItemIds)

				syncSourceOutgoingEdges(node, totalCount, refreshedFromBackend, removedItemIds)
			}

			if (droppedSinkIds.length > 0) {
				clearDroppedSinkNodes(new Set(updatedDetail.sink_ids || []))
			}

			syncDownstreamItemCounts()
		}

		async function updatePipelineSubset(
			kind: 'hosts' | 'sinks',
			nextIds: string[],
			force: boolean = false
		): Promise<boolean> {
			if (loadingResources.updatingSubset) {
				return false
			}
			if (!pipelineId.value || !pipelineDetail.value) {
				selectorError.value = 'Pipeline details are not loaded yet.'
				return false
			}

			const nextSet = new Set(nextIds)
			const normalizedNextIds = Array.from(nextSet)
			if (kind === 'hosts' && normalizedNextIds.length === 0) {
				selectorError.value = 'At least one host must be selected.'
				return false
			}

			const currentIds =
				kind === 'hosts'
					? pipelineDetail.value.host_ids || []
					: pipelineDetail.value.sink_ids || []
			const currentSet = new Set(currentIds)
			const addedIds = normalizedNextIds.filter((id) => !currentSet.has(id))
			const droppedIds = currentIds.filter((id) => !nextSet.has(id))

			if (addedIds.length === 0 && droppedIds.length === 0) {
				subsetConfirmation.value.isOpen = false
				selectorError.value = null
				if (kind === 'hosts') {
					hostSelectorModalOpen.value = false
				} else {
					sinkSelectorModalOpen.value = false
				}
				return true
			}

			if (!force && droppedIds.length > 0) {
				const localUsage = findLocalSubsetUsage(kind, droppedIds)
				if (localUsage.itemCount > 0 || localUsage.sinkNodeCount > 0) {
					subsetConfirmation.value = {
						isOpen: true,
						kind,
						nextIds: normalizedNextIds,
						message: formatSubsetInUseMessage(
							localUsage.itemCount,
							localUsage.sinkNodeCount
						)
					}
					return false
				}
			}

			loadingResources.updatingSubset = true
			selectorError.value = null
			const cUrls = clusterUrl.value ? [clusterUrl.value] : undefined

			try {
				const payload: UpdatePipelineRequest = {
					name: pipelineDetail.value.name,
					adm_status: pipelineDetail.value.adm_status,
					visibility: pipelineDetail.value.visibility,
					host_ids:
						kind === 'hosts'
							? normalizedNextIds
							: [...(pipelineDetail.value.host_ids || [])],
					sink_ids:
						kind === 'sinks'
							? normalizedNextIds
							: [...(pipelineDetail.value.sink_ids || [])],
					last_updated: pipelineDetail.value.last_updated,
					force
				}

				const res = await pipelinesApi.updatePipeline(
					pipelineId.value,
					payload,
					cUrls,
					force ? undefined : { suppressErrorStatuses: [409] }
				)
				if (!res?.success || !res.data) {
					selectorError.value = 'Failed to update pipeline resources.'
					return false
				}

				await syncCanvasAfterResourceUpdate(
					res.data,
					kind === 'hosts' ? droppedIds : [],
					kind === 'sinks' ? droppedIds : [],
					force
				)

				subsetConfirmation.value.isOpen = false
				if (kind === 'hosts') {
					hostSelectorModalOpen.value = false
				} else {
					sinkSelectorModalOpen.value = false
				}
				return true
			} catch (err: any) {
				const errPayload = err?.data?.error
				if (err?.status === 409 && errPayload?.data && !force) {
					const itemCount = Number(errPayload.data.item_count || 0)
					const nodeCount = Array.isArray(errPayload.data.node_ids)
						? errPayload.data.node_ids.length
						: 0
					subsetConfirmation.value = {
						isOpen: true,
						kind,
						nextIds: normalizedNextIds,
						message: formatSubsetInUseMessage(
							itemCount,
							kind === 'sinks' ? nodeCount : 0
						)
					}
					return false
				}

				if (err?.status === 409) {
					try {
						const refreshed = await pipelinesApi.getPipelineById(
							pipelineId.value,
							cUrls
						)
						if (refreshed?.success && refreshed.data) {
							pipelineDetail.value = refreshed.data
							syncCanvasTimestampFromDetail(refreshed.data)
						}
					} catch {
						// Keep existing pipelineDetail if refresh fails
					}
				}

				selectorError.value =
					errPayload?.detail || err?.message || 'Failed to update pipeline resources.'
				return false
			} finally {
				loadingResources.updatingSubset = false
			}
		}

		async function applyPipelineHosts(hostIds: string[]): Promise<boolean> {
			return updatePipelineSubset('hosts', hostIds, false)
		}

		async function applyPipelineSinks(sinkIds: string[]): Promise<boolean> {
			return updatePipelineSubset('sinks', sinkIds, false)
		}

		async function confirmSubsetForceUpdate(): Promise<boolean> {
			return updatePipelineSubset(
				subsetConfirmation.value.kind,
				subsetConfirmation.value.nextIds,
				true
			)
		}

		async function saveGraph(): Promise<boolean> {
			const outcome = await executeSaveGraph()
			return outcome.ok
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
			hasSourceNode,
			isDirty,
			loading,
			error,
			successMessage,
			availableHosts,
			availableSinks,
			hostById,
			sinkById,
			pipelineItems,
			loadingItems,
			hostSelectorModalOpen,
			sinkSelectorModalOpen,
			selectorError,
			loadingResources,
			subsetConfirmation,
			openHostSelectorModal,
			closeHostSelectorModal,
			openSinkSelectorModal,
			closeSinkSelectorModal,
			clearSelectorError,
			cancelSubsetConfirmation,
			applyPipelineHosts,
			applyPipelineSinks,
			confirmSubsetForceUpdate,
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
			syncDownstreamItemCounts,
			buildGraphRepresentation
		}
	},
	{ persist: false }
)

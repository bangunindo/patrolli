<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { FormMode } from '@@/shared/types/app.type'
import AppInputText from '@/components/app/input/AppInputText.vue'
import AppInputSink from '@/components/app/input/AppInputSink.vue'
import sinksApi from '@/app/api/sinks.api'
import type { ISink } from '@@/shared/types/sink.type'
import { formatUtcDate } from '@/app/helpers/date.helper'
import { usePipelineBuildStore } from '@/stores/pipeline.builder.store'
import { useSinksStore } from '@/stores/sinks.store'

const store = usePipelineBuildStore()
const sinksStore = useSinksStore()

const isOpen = computed(() => {
	const node = store.selectedNode as any
	return !!node && (node.type === 'source' || node.type === 'sink')
})
const currentNode = computed(() => store.selectedNode as any)

const nodeName = ref('')

// Sink Selection & Details
const selectedSinkId = ref('')
const activeSinkDetail = ref<ISink | null>(null)
const loadingSinkDetail = ref(false)

const clusterURLs = computed(() => (store.clusterUrl ? [store.clusterUrl] : undefined))

async function fetchActiveSinkDetails(sinkId?: string) {
	if (!sinkId) {
		activeSinkDetail.value = null
		return
	}
	loadingSinkDetail.value = true
	try {
		const res = await sinksApi.getSinkById(sinkId, clusterURLs.value)
		if (res?.success && res.data) {
			activeSinkDetail.value = res.data
		} else {
			activeSinkDetail.value =
				(store.availableSinks.find((s) => s.sink_id === sinkId) as any) || null
		}
	} catch {
		activeSinkDetail.value =
			(store.availableSinks.find((s) => s.sink_id === sinkId) as any) || null
	} finally {
		loadingSinkDetail.value = false
	}
}

// Source Node Accordion state
const isHostsOpen = ref(true)
const isItemsOpen = ref(true)
const itemSearchQuery = ref('')

watch(
	() => store.selectedNode,
	(node: any) => {
		if (node) {
			nodeName.value = node.data?.name || node.label || 'Node'

			if (node.type === 'sink') {
				const sId = node.data?.sinkId || node.data?.sink_id || ''
				selectedSinkId.value = sId
				fetchActiveSinkDetails(sId)
			} else {
				selectedSinkId.value = ''
				activeSinkDetail.value = null
			}
		}
	},
	{ immediate: true }
)

// Pipeline Hosts
const pipelineHosts = computed(() => {
	const hostIds = store.pipelineDetail?.host_ids || []
	return store.availableHosts.filter((h) => hostIds.includes(h.host_id))
})

// Pipeline Associated Items (filtered by search query)
const filteredAssociatedItems = computed(() => {
	let list = store.pipelineItems || []
	if (itemSearchQuery.value.trim()) {
		const q = itemSearchQuery.value.toLowerCase()
		list = list.filter((i) => {
			const name = (i.name || i.resolved_name || '').toLowerCase()
			const hName = (i.host_name || '').toLowerCase()
			return name.includes(q) || hName.includes(q)
		})
	}
	return list
})

// Sinks registered in the current pipeline
const pipelineSinkIds = computed(() => {
	return store.pipelineDetail?.sink_ids || []
})

// Currently selected sink object metadata
const activeSinkMeta = computed(() => {
	if (!selectedSinkId.value) return null
	return store.availableSinks.find((s) => s.sink_id === selectedSinkId.value) || null
})

function onSinkSelectChange(val?: string | string[]) {
	const sinkId = Array.isArray(val) ? val[0] : val
	if (sinkId) {
		selectedSinkId.value = sinkId
	}
	if (currentNode.value && selectedSinkId.value) {
		store.selectSinkForNode(currentNode.value.id, selectedSinkId.value)
		fetchActiveSinkDetails(selectedSinkId.value)
		if (activeSinkMeta.value) {
			nodeName.value = activeSinkMeta.value.name || 'Sink Target'
		}
	}
}

const parsedLabels = computed(() => {
	if (!activeSinkDetail.value?.labels) return {}
	try {
		return typeof activeSinkDetail.value.labels === 'string'
			? JSON.parse(activeSinkDetail.value.labels)
			: activeSinkDetail.value.labels
	} catch {
		return {}
	}
})

const hasLabels = computed(() => {
	return Object.keys(parsedLabels.value).length > 0
})

function handleOpenCreateSink() {
	sinksStore.toggleSinkModalForm(true, FormMode.ADD)
}

function closeDrawer() {
	store.selectedNode = undefined
}

function handleDeleteNode() {
	if (currentNode.value?.id) {
		store.removeNode(currentNode.value.id)
	}
}
</script>

<template>
	<!-- Flyout Drawer Container matching Figma #1132:81396 unified panel -->
	<aside
		v-if="isOpen"
		class="absolute top-[66px] right-0 bottom-0 w-[420px] bg-white dark:bg-slate-900 border-l border-t border-slate-200 dark:border-slate-800 shadow-2xl z-30 flex flex-col animate-in slide-in-from-right duration-200 nokey nowheel nodrag nopan"
	>
		<!-- Drawer Header -->
		<div
			class="flex items-center justify-between px-5 py-4 border-b border-slate-200 dark:border-slate-800"
		>
			<div>
				<h2 class="text-sm font-bold text-slate-900 dark:text-slate-100">
					Node Configuration
				</h2>
				<span class="text-[11px] text-slate-400 capitalize">
					{{ currentNode?.type }} Node (Unified Details & Settings)
				</span>
			</div>
			<UiButton
				variant="ghost"
				size="icon"
				class="h-7 w-7 text-muted-foreground hover:text-foreground"
				@click="closeDrawer"
			>
				<UiIcon name="close" size="14px" />
			</UiButton>
		</div>

		<!-- Drawer Body -->
		<div class="flex-1 p-5 overflow-y-auto flex flex-col gap-5">
			<!-- Section: Basic Information -->
			<div class="flex flex-col gap-3">
				<div class="flex items-center gap-1.5">
					<div class="w-2 h-2 rounded-full bg-sky-600" />
					<span class="text-xs font-bold uppercase tracking-wider text-sky-600">
						1. Node Details
					</span>
				</div>

				<div class="flex flex-col gap-1.5">
					<label class="text-xs font-medium text-slate-700 dark:text-slate-300">
						Node Name
					</label>
					<AppInputText v-model="nodeName" placeholder="Node Name" readonly />
				</div>
			</div>

			<!-- ============================================== -->
			<!-- SOURCE NODE DETAILS (ACCORDION HOSTS & ITEMS)  -->
			<!-- ============================================== -->
			<div
				v-if="currentNode?.type === 'source'"
				class="flex flex-col gap-4 pt-4 border-t border-slate-100 dark:border-slate-800"
			>
				<div class="flex items-center gap-1.5">
					<div class="w-2 h-2 rounded-full bg-sky-600" />
					<span class="text-xs font-bold uppercase tracking-wider text-sky-600">
						2. Pipeline Source Feeds
					</span>
				</div>

				<!-- Accordion 1: Selected Hosts -->
				<div
					class="border border-slate-200 dark:border-slate-800 rounded-lg overflow-hidden"
				>
					<button
						type="button"
						class="w-full px-3 py-2 bg-slate-50 dark:bg-slate-800/60 flex items-center justify-between text-xs font-semibold text-slate-800 dark:text-slate-200 hover:bg-slate-100 dark:hover:bg-slate-800 transition-colors"
						@click="isHostsOpen = !isHostsOpen"
					>
						<span class="flex items-center gap-2">
							<svg
								width="14"
								height="14"
								viewBox="0 0 24 24"
								fill="none"
								stroke="#0284C7"
								stroke-width="2"
								stroke-linecap="round"
								stroke-linejoin="round"
							>
								<rect x="2" y="2" width="20" height="8" rx="2" ry="2" />
								<rect x="2" y="14" width="20" height="8" rx="2" ry="2" />
								<line x1="6" y1="6" x2="6.01" y2="6" />
								<line x1="6" y1="18" x2="6.01" y2="18" />
							</svg>
							Selected Hosts ({{ pipelineHosts.length }})
						</span>
						<span class="text-slate-400 text-[10px]">
							{{ isHostsOpen ? '▲' : '▼' }}
						</span>
					</button>

					<div
						v-show="isHostsOpen"
						class="p-3 bg-white dark:bg-slate-900 flex flex-col gap-2"
					>
						<div
							v-if="pipelineHosts.length === 0"
							class="text-xs text-slate-400 text-center py-2 italic"
						>
							No hosts assigned to this pipeline yet.
						</div>
						<div
							v-for="host in pipelineHosts"
							:key="host.host_id"
							class="flex items-center justify-between p-2 rounded bg-slate-50 dark:bg-slate-800/40 border border-slate-100 dark:border-slate-800"
						>
							<div class="flex items-center gap-2 min-w-0">
								<span class="w-2 h-2 rounded-full bg-emerald-500 shrink-0" />
								<div class="flex flex-col min-w-0">
									<span
										class="text-xs font-medium text-slate-800 dark:text-slate-200 truncate"
									>
										{{ host.name || host.host_id }}
									</span>
									<span class="text-[10px] text-slate-400 truncate">
										{{ host.endpoint || host.host_id }}
									</span>
								</div>
							</div>
							<span
								class="text-[10px] font-medium px-1.5 py-0.5 rounded bg-sky-50 dark:bg-sky-950/60 text-sky-600 dark:text-sky-400 shrink-0"
							>
								Host
							</span>
						</div>
					</div>
				</div>

				<!-- Accordion 2: Associated Items -->
				<div
					class="border border-slate-200 dark:border-slate-800 rounded-lg overflow-hidden"
				>
					<button
						type="button"
						class="w-full px-3 py-2 bg-slate-50 dark:bg-slate-800/60 flex items-center justify-between text-xs font-semibold text-slate-800 dark:text-slate-200 hover:bg-slate-100 dark:hover:bg-slate-800 transition-colors"
						@click="isItemsOpen = !isItemsOpen"
					>
						<span class="flex items-center gap-2">
							<svg
								width="14"
								height="14"
								viewBox="0 0 24 24"
								fill="none"
								stroke="#0284C7"
								stroke-width="2"
								stroke-linecap="round"
								stroke-linejoin="round"
							>
								<polygon points="12 2 2 7 12 12 22 7 12 2" />
								<polyline points="2 17 12 22 22 17" />
								<polyline points="2 12 12 17 22 12" />
							</svg>
							Associated Items ({{ filteredAssociatedItems.length }})
						</span>
						<span class="text-slate-400 text-[10px]">
							{{ isItemsOpen ? '▲' : '▼' }}
						</span>
					</button>

					<div
						v-show="isItemsOpen"
						class="p-3 bg-white dark:bg-slate-900 flex flex-col gap-2.5"
					>
						<!-- Item Search -->
						<AppInputText
							v-model="itemSearchQuery"
							placeholder="Search items by name..."
						/>

						<div
							class="max-h-52 overflow-y-auto divide-y divide-slate-100 dark:divide-slate-800"
						>
							<div
								v-if="filteredAssociatedItems.length === 0"
								class="text-xs text-slate-400 text-center py-4 italic"
							>
								No associated items found.
							</div>
							<div
								v-for="item in filteredAssociatedItems"
								:key="item.item_id"
								class="py-1.5 flex items-center justify-between gap-2"
							>
								<div class="flex flex-col min-w-0">
									<span
										class="text-xs font-medium text-slate-800 dark:text-slate-200 truncate"
									>
										{{ item.name || item.resolved_name || item.item_id }}
									</span>
									<span class="text-[10px] text-slate-400 truncate">
										{{ item.host_name || item.host_id }}
									</span>
								</div>
								<span
									v-if="item.type"
									class="px-1.5 py-0.5 rounded text-[10px] bg-slate-100 dark:bg-slate-800 text-slate-600 dark:text-slate-300 shrink-0"
								>
									{{ item.type }}
								</span>
							</div>
						</div>
					</div>
				</div>
			</div>

			<!-- ============================================== -->
			<!-- SINK NODE CONFIGURATION (SELECTOR & DETAILS)   -->
			<!-- ============================================== -->
			<div
				v-else-if="currentNode?.type === 'sink'"
				class="flex flex-col gap-4 pt-4 border-t border-slate-100 dark:border-slate-800"
			>
				<div class="flex items-center gap-1.5">
					<div class="w-2 h-2 rounded-full bg-indigo-600" />
					<span class="text-xs font-bold uppercase tracking-wider text-indigo-600">
						2. Sink Target Selection
					</span>
				</div>

				<!-- Sink Dropdown Selector -->
				<div class="flex flex-col gap-1.5">
					<label class="text-xs font-medium text-slate-700 dark:text-slate-300">
						Registered Sink
					</label>

					<div v-if="pipelineSinkIds.length > 0">
						<AppInputSink
							v-model="selectedSinkId"
							:multiple="false"
							:sink-ids="pipelineSinkIds"
							:cluster-url="store.clusterUrl"
							:show-label="false"
							label="Sink"
							@update:model-value="onSinkSelectChange"
						/>
					</div>

					<!-- Empty State: No sinks registered in pipeline -->
					<div
						v-else
						class="p-3 rounded-lg border border-amber-200 dark:border-amber-900/60 bg-amber-50/70 dark:bg-amber-950/30 flex flex-col gap-2"
					>
						<p class="text-xs text-amber-800 dark:text-amber-200 font-medium">
							No sinks registered in this pipeline.
						</p>
						<p class="text-[11px] text-amber-700/80 dark:text-amber-300/80">
							Add a sink to your cluster and attach it to this pipeline configuration
							to route events.
						</p>
						<button
							type="button"
							class="mt-1 px-3 py-1.5 text-xs rounded bg-indigo-600 hover:bg-indigo-700 text-white font-medium shadow-xs self-start"
							@click="handleOpenCreateSink"
						>
							+ Create / Add Sink
						</button>
					</div>
				</div>

				<!-- Selected Sink Details matching Configuration/Sinks ModalDetail -->
				<div v-if="selectedSinkId" class="flex flex-col gap-3 pt-2">
					<div class="text-xs font-bold text-slate-700 dark:text-slate-200">
						Sink Target Details
					</div>

					<div v-if="loadingSinkDetail" class="flex items-center justify-center py-6">
						<UiIcon
							name="loading"
							class="animate-spin text-muted-foreground"
							size="18px"
						/>
					</div>

					<div
						v-else-if="activeSinkDetail"
						class="flex flex-col gap-3 p-3 rounded-lg bg-slate-50 dark:bg-slate-800/40 border border-slate-200 dark:border-slate-800"
					>
						<div class="flex flex-col gap-4">
							<div class="flex flex-row items-center">
								<div class="text-sm font-medium w-[120px]">Sink ID</div>
								<div class="text-sm font-normal">
									: {{ activeSinkDetail.sink_id }}
								</div>
							</div>

							<div class="flex flex-row items-center">
								<div class="text-sm font-medium w-[120px]">Sink Name</div>
								<div class="text-sm font-normal">: {{ activeSinkDetail.name }}</div>
							</div>

							<div class="flex flex-row items-center">
								<div class="text-sm font-medium w-[120px]">Status</div>
								<div class="text-sm font-normal flex items-center gap-2">
									:
									<div
										class="w-2.5 h-2.5 rounded-full shrink-0"
										:class="
											Number(activeSinkDetail.adm_status) === 1
												? 'bg-emerald-500'
												: 'bg-rose-500'
										"
									></div>
									{{
										Number(activeSinkDetail.adm_status) === 1
											? 'Active'
											: 'Inactive'
									}}
								</div>
							</div>

							<div class="flex flex-row items-center">
								<div class="text-sm font-medium w-[120px]">Worker Count</div>
								<div class="text-sm font-normal">
									: {{ activeSinkDetail.worker_count ?? 0 }}
								</div>
							</div>

							<div class="flex flex-row items-center">
								<div class="text-sm font-medium w-[120px]">Input Count</div>
								<div class="text-sm font-normal">
									: {{ (activeSinkDetail as any).input_count ?? 0 }}
								</div>
							</div>

							<div class="flex flex-row items-center">
								<div class="text-sm font-medium w-[120px]">Last Updated</div>
								<div class="text-sm font-normal">
									: {{ formatUtcDate(activeSinkDetail.last_updated) }}
								</div>
							</div>

							<div class="flex flex-row items-center">
								<div class="text-sm font-medium w-[120px]">Type</div>
								<div class="text-sm font-normal">
									:
									{{
										activeSinkDetail.config?.type ||
										(activeSinkDetail as any).type ||
										'-'
									}}
								</div>
							</div>

							<div class="flex flex-col gap-2">
								<div class="text-sm font-medium w-[120px]">Type Config</div>
								<div
									class="text-sm font-normal bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-700 p-2 rounded max-h-40 overflow-auto whitespace-pre-wrap font-mono"
								>
									{{
										(activeSinkDetail.config as any)?.config
											? JSON.stringify(
													(activeSinkDetail.config as any).config,
													null,
													2
												)
											: activeSinkDetail.config
												? JSON.stringify(activeSinkDetail.config, null, 2)
												: '-'
									}}
								</div>
							</div>

							<div class="flex flex-col gap-2">
								<div class="text-sm font-medium w-[120px]">Batch</div>
								<div
									class="text-sm font-normal bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-700 p-2 rounded max-h-40 overflow-auto whitespace-pre-wrap font-mono"
								>
									{{
										activeSinkDetail.batch
											? JSON.stringify(activeSinkDetail.batch, null, 2)
											: '-'
									}}
								</div>
							</div>

							<div class="flex flex-col gap-2">
								<div class="text-sm font-medium w-[120px]">Channel</div>
								<div
									class="text-sm font-normal bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-700 p-2 rounded max-h-40 overflow-auto whitespace-pre-wrap font-mono"
								>
									{{
										activeSinkDetail.channel
											? JSON.stringify(activeSinkDetail.channel, null, 2)
											: '-'
									}}
								</div>
							</div>

							<div
								class="flex"
								:class="!hasLabels ? 'flex-row items-center' : 'flex-col gap-2'"
							>
								<div class="text-sm font-medium w-[120px] shrink-0">Labels</div>
								<div class="flex gap-2 flex-wrap">
									<template v-if="hasLabels">
										<UiBadge
											v-for="(value, key) in parsedLabels"
											:key="`detail-labels-${key}-${value}`"
											variant="outline"
											class="text-xs font-normal"
										>
											{{ key }} : {{ value }}
										</UiBadge>
									</template>
									<div v-else class="text-sm font-normal">: -</div>
								</div>
							</div>
						</div>
					</div>
				</div>
			</div>
		</div>

		<!-- Drawer Footer -->
		<div
			v-if="currentNode?.type !== 'source'"
			class="p-4 border-t border-slate-200 dark:border-slate-800 flex items-center justify-between bg-slate-50/50 dark:bg-slate-950/40"
		>
			<UiButton variant="destructive" size="sm" @click="handleDeleteNode">
				Delete Node
			</UiButton>
		</div>
	</aside>
</template>

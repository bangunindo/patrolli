<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import {
	Alert as UiAlert,
	AlertDescription as UiAlertDescription,
	AlertTitle as UiAlertTitle
} from '@/components/ui/alert'
import { Badge as UiBadge } from '@/components/ui/badge'
import { Button as UiButton } from '@/components/ui/button'
import { Card as UiCard, CardContent as UiCardContent } from '@/components/ui/card'
import {
	Collapsible as UiCollapsible,
	CollapsibleContent as UiCollapsibleContent,
	CollapsibleTrigger as UiCollapsibleTrigger
} from '@/components/ui/collapsible'
import UiIcon from '@/components/ui/icon/Icon.vue'
import { ScrollArea as UiScrollArea } from '@/components/ui/scroll-area'
import { Separator as UiSeparator } from '@/components/ui/separator'
import AppInputText from '@/components/app/input/AppInputText.vue'
import AppInputSink from '@/components/app/input/AppInputSink.vue'
import sinksApi from '@/app/api/sinks.api'
import type { ISink } from '@@/shared/types/sink.type'
import { formatUtcDate } from '@/app/helpers/date.helper'
import { usePipelineBuildStore } from '@/stores/pipeline.builder.store'

const store = usePipelineBuildStore()

const isOpen = computed(() => {
	const node = store.selectedNode as any
	return !!node && (node.type === 'source' || node.type === 'sink')
})
const currentNode = computed(() => store.selectedNode as any)

const nodeName = ref('')

const selectedSinkId = ref('')
const activeSinkDetail = ref<ISink | null>(null)
const loadingSinkDetail = ref(false)
const lastFetchedSinkId = ref<string | null>(null)
let activeRequestSinkId: string | null = null

const clusterURLs = computed(() => (store.clusterUrl ? [store.clusterUrl] : undefined))

function resetSinkDetailState() {
	activeRequestSinkId = null
	selectedSinkId.value = ''
	activeSinkDetail.value = null
	lastFetchedSinkId.value = null
	loadingSinkDetail.value = false
}

async function fetchActiveSinkDetails(sinkId?: string) {
	if (!sinkId) {
		resetSinkDetailState()
		return
	}
	if (
		(loadingSinkDetail.value && activeRequestSinkId === sinkId) ||
		(lastFetchedSinkId.value === sinkId && activeSinkDetail.value?.sink_id === sinkId)
	) {
		return
	}
	activeRequestSinkId = sinkId
	loadingSinkDetail.value = true
	try {
		const res = await sinksApi.getSinkById(sinkId, clusterURLs.value)
		if (activeRequestSinkId !== sinkId) return
		if (res?.success && res.data) {
			activeSinkDetail.value = res.data
			lastFetchedSinkId.value = sinkId
		} else {
			lastFetchedSinkId.value = null
			activeSinkDetail.value = (store.sinkById.get(sinkId) as any) || null
		}
	} catch {
		if (activeRequestSinkId !== sinkId) return
		lastFetchedSinkId.value = null
		activeSinkDetail.value = (store.sinkById.get(sinkId) as any) || null
	} finally {
		if (activeRequestSinkId === sinkId) {
			loadingSinkDetail.value = false
		}
	}
}

const isHostsOpen = ref(true)
const isItemsOpen = ref(true)
const itemSearchQuery = ref('')

watch(
	() =>
		[
			store.selectedNode,
			store.selectedNode?.data?.sinkId,
			store.pipelineDetail?.sink_ids
		] as const,
	([node]) => {
		if (node) {
			nodeName.value = node.data?.name || node.label || 'Node'

			if (node.type === 'sink') {
				const sId = node.data?.sinkId || node.data?.sink_id || ''
				if (!sId) {
					resetSinkDetailState()
				} else if (
					sId !== selectedSinkId.value ||
					(lastFetchedSinkId.value !== sId &&
						!(loadingSinkDetail.value && activeRequestSinkId === sId))
				) {
					selectedSinkId.value = sId
					fetchActiveSinkDetails(sId)
				}
			} else {
				resetSinkDetailState()
			}
		}
	},
	{ immediate: true }
)

const pipelineHosts = computed(() => {
	const hostIds = store.pipelineDetail?.host_ids
	if (!hostIds || hostIds.length === 0) return []
	const lookup = store.hostById
	return hostIds.map((id) => lookup.get(id) || { host_id: id, name: id })
})

function getItemHostName(item: any): string {
	return (
		item?.host_name ||
		(item?.host_id && store.hostById.get(item.host_id)?.name) ||
		item?.host_id ||
		''
	)
}

const filteredAssociatedItems = computed(() => {
	const list = store.pipelineItems || []
	const q = itemSearchQuery.value.trim().toLowerCase()
	if (!q) return list
	return list.filter((i) => {
		const name = (i.name || i.resolved_name || '').toLowerCase()
		const hName = getItemHostName(i).toLowerCase()
		return name.includes(q) || hName.includes(q)
	})
})

const pipelineSinkIds = computed(() => {
	return store.pipelineDetail?.sink_ids || []
})

function onSinkSelectChange(val?: string | string[]) {
	const sinkId = Array.isArray(val) ? val[0] : val
	if (sinkId) {
		selectedSinkId.value = sinkId
	}
	if (currentNode.value && selectedSinkId.value) {
		store.selectSinkForNode(currentNode.value.id, selectedSinkId.value)
		fetchActiveSinkDetails(selectedSinkId.value)
		const sinkMeta = store.sinkById.get(selectedSinkId.value)
		if (sinkMeta) {
			nodeName.value = sinkMeta.name || 'Sink Target'
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

		<div class="flex-1 p-5 overflow-y-auto flex flex-col gap-5">
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

			<div v-if="currentNode?.type === 'source'" class="flex flex-col gap-4">
				<UiSeparator />

				<div class="flex items-center gap-1.5">
					<div class="w-2 h-2 rounded-full bg-sky-600" />
					<span class="text-xs font-bold uppercase tracking-wider text-sky-600">
						2. Pipeline Source Feeds
					</span>
				</div>

				<UiCollapsible
					v-model:open="isHostsOpen"
					class="border border-border rounded-lg overflow-hidden"
				>
					<UiCollapsibleTrigger as-child>
						<UiButton
							variant="ghost"
							class="w-full px-3 py-2 h-auto rounded-none bg-muted/50 hover:bg-muted flex items-center justify-between text-xs font-semibold text-foreground"
						>
							<span class="flex items-center gap-2">
								<UiIcon name="server-network" size="14px" class="text-sky-600" />
								Selected Hosts ({{ pipelineHosts.length }})
							</span>
							<UiIcon
								:name="isHostsOpen ? 'chevron-up' : 'chevron-down'"
								size="14px"
								class="text-muted-foreground"
							/>
						</UiButton>
					</UiCollapsibleTrigger>

					<UiCollapsibleContent class="p-3 bg-card flex flex-col gap-2">
						<div
							v-if="pipelineHosts.length === 0"
							class="text-xs text-muted-foreground text-center py-2 italic"
						>
							No hosts assigned to this pipeline yet.
						</div>
						<div
							v-for="host in pipelineHosts"
							:key="host.host_id"
							class="flex items-center justify-between p-2 rounded bg-muted/40 border border-border"
						>
							<div class="flex items-center gap-2 min-w-0">
								<span class="w-2 h-2 rounded-full bg-emerald-500 shrink-0" />
								<div class="flex flex-col min-w-0">
									<span class="text-xs font-medium text-foreground truncate">
										{{ host.name || host.host_id }}
									</span>
									<span class="text-[10px] text-muted-foreground truncate">
										{{ host.endpoint || host.host_id }}
									</span>
								</div>
							</div>
							<UiBadge
								variant="secondary"
								class="text-[10px] font-medium px-1.5 py-0.5 bg-sky-50 dark:bg-sky-950/60 text-sky-600 dark:text-sky-400 shrink-0"
							>
								Host
							</UiBadge>
						</div>
					</UiCollapsibleContent>
				</UiCollapsible>

				<UiCollapsible
					v-model:open="isItemsOpen"
					class="border border-border rounded-lg overflow-hidden"
				>
					<UiCollapsibleTrigger as-child>
						<UiButton
							variant="ghost"
							class="w-full px-3 py-2 h-auto rounded-none bg-muted/50 hover:bg-muted flex items-center justify-between text-xs font-semibold text-foreground"
						>
							<span class="flex items-center gap-2">
								<UiIcon name="layers-outline" size="14px" class="text-sky-600" />
								Associated Items ({{ filteredAssociatedItems.length }})
							</span>
							<UiIcon
								:name="isItemsOpen ? 'chevron-up' : 'chevron-down'"
								size="14px"
								class="text-muted-foreground"
							/>
						</UiButton>
					</UiCollapsibleTrigger>

					<UiCollapsibleContent class="p-3 bg-card flex flex-col gap-2.5">
						<AppInputText
							v-model="itemSearchQuery"
							placeholder="Search items by name..."
						/>

						<UiScrollArea class="h-52">
							<div class="divide-y divide-border pr-2">
								<div
									v-if="filteredAssociatedItems.length === 0"
									class="text-xs text-muted-foreground text-center py-4 italic"
								>
									No associated items found.
								</div>
								<div
									v-for="item in filteredAssociatedItems"
									:key="item.item_id"
									class="py-1.5 flex items-center justify-between gap-2"
								>
									<div class="flex flex-col min-w-0">
										<span class="text-xs font-medium text-foreground truncate">
											{{ item.name || item.resolved_name || item.item_id }}
										</span>
										<span class="text-[10px] text-muted-foreground truncate">
											{{ getItemHostName(item) }}
										</span>
									</div>
									<UiBadge
										v-if="item.type"
										variant="secondary"
										class="px-1.5 py-0.5 text-[10px] font-normal shrink-0"
									>
										{{ item.type }}
									</UiBadge>
								</div>
							</div>
						</UiScrollArea>
					</UiCollapsibleContent>
				</UiCollapsible>
			</div>

			<div v-else-if="currentNode?.type === 'sink'" class="flex flex-col gap-4">
				<UiSeparator />

				<div class="flex items-center gap-1.5">
					<div class="w-2 h-2 rounded-full bg-indigo-600" />
					<span class="text-xs font-bold uppercase tracking-wider text-indigo-600">
						2. Sink Target Selection
					</span>
				</div>

				<div class="flex flex-col gap-1.5">
					<div class="flex items-center justify-between gap-2">
						<label class="text-xs font-medium text-foreground/80">
							Registered Sink
						</label>
						<UiButton
							v-if="pipelineSinkIds.length > 0"
							variant="link"
							size="xs"
							class="h-auto p-0 text-[11px] font-medium"
							@click="store.openSinkSelectorModal()"
						>
							Manage Sinks
						</UiButton>
					</div>

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

					<UiAlert
						v-else
						class="p-3 border-amber-200 dark:border-amber-900/60 bg-amber-50/70 dark:bg-amber-950/30 flex flex-col gap-2"
					>
						<UiAlertTitle
							class="text-xs text-amber-800 dark:text-amber-200 font-medium m-0"
						>
							No sinks registered in this pipeline.
						</UiAlertTitle>
						<UiAlertDescription
							class="text-[11px] text-amber-700/80 dark:text-amber-300/80"
						>
							Add a sink to your cluster and attach it to this pipeline configuration
							to route events.
						</UiAlertDescription>
						<UiButton
							size="xs"
							class="mt-1 bg-indigo-600 hover:bg-indigo-700 text-white self-start"
							@click="store.openSinkSelectorModal()"
						>
							+ Select Sinks
						</UiButton>
					</UiAlert>
				</div>

				<div v-if="selectedSinkId" class="flex flex-col gap-3 pt-2">
					<div class="text-xs font-bold text-foreground">Sink Target Details</div>

					<div v-if="loadingSinkDetail" class="flex items-center justify-center py-6">
						<UiIcon
							name="loading"
							class="animate-spin text-muted-foreground"
							size="18px"
						/>
					</div>

					<UiCard v-else-if="activeSinkDetail" class="bg-muted/30 shadow-none">
						<UiCardContent class="p-3 flex flex-col gap-3">
							<div class="flex flex-col gap-4">
								<div class="flex flex-row items-center">
									<div class="text-sm font-medium w-[120px]">Sink ID</div>
									<div class="text-sm font-normal">
										: {{ activeSinkDetail.sink_id }}
									</div>
								</div>

								<div class="flex flex-row items-center">
									<div class="text-sm font-medium w-[120px]">Sink Name</div>
									<div class="text-sm font-normal">
										: {{ activeSinkDetail.name }}
									</div>
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
													? JSON.stringify(
															activeSinkDetail.config,
															null,
															2
														)
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
						</UiCardContent>
					</UiCard>
				</div>
			</div>
		</div>

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

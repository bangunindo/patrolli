<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { storeToRefs } from 'pinia'
import {
	getProcessorModuleBadgeClass,
	usePipelineBuildStore
} from '@/stores/pipeline.builder.store'
import type { IGroupProcessorItem } from '@/stores/pipeline.builder.store'
import { useAppStore } from '@/stores/app.store'
import { AppWebSocket } from '@/app/services/websocket.service'
import appHelper from '@/app/helpers/app.helper'

interface ICapturedRecord {
	id: number
	time: string
	status: string
	statusType: 'success' | 'danger' | 'warning'
	message_in: any
	message_out: any
}

const store = usePipelineBuildStore()
const appStore = useAppStore()
const { activeClusterURL } = storeToRefs(appStore)

const modalState = computed(() => store.previewDataModal)
const isOpen = computed(() => modalState.value.isOpen)

const currentNode = computed(() => store.selectedNode as any)
const nodeId = computed(() => currentNode.value?.id)
const processorsList = computed<IGroupProcessorItem[]>(() => {
	const list = currentNode.value?.data?.processors
	if (list && list.length > 0) return list
	return []
})

const selectedProcessorId = ref<string>('')
const stopAfterSeconds = ref(10)
const remainingSeconds = ref(10)
const isPooling = ref(false)
const loading = ref(false)
const capturedRecords = ref<ICapturedRecord[]>([])
const selectedRecordIndex = ref(0)
const viewMode = ref<'list' | 'detail'>('list')

const selectedProcessorIds = ref<string[]>([])
const activeMenuProcessorId = ref<string | null>(null)

let timer: ReturnType<typeof setInterval> | null = null
let noDataTimer: ReturnType<typeof setTimeout> | undefined
let switchTimeout: ReturnType<typeof setTimeout> | undefined
let activeConnectionToken = 0
let ws: AppWebSocket | undefined

const activeRecord = computed<ICapturedRecord | undefined>(() => {
	return capturedRecords.value[selectedRecordIndex.value] || capturedRecords.value[0]
})

const selectedProcessor = computed(() => {
	return processorsList.value.find((p) => p.processor_id === selectedProcessorId.value)
})

const allProcessorsSelected = computed(() => {
	if (processorsList.value.length === 0) return false
	return processorsList.value.every((p) => selectedProcessorIds.value.includes(p.processor_id))
})

function toggleSelectAll(checked: boolean) {
	if (checked) {
		selectedProcessorIds.value = processorsList.value.map((p) => p.processor_id)
	} else {
		selectedProcessorIds.value = []
	}
}

function toggleSelectProcessor(id: string, event: Event) {
	event.stopPropagation()
	if (selectedProcessorIds.value.includes(id)) {
		selectedProcessorIds.value = selectedProcessorIds.value.filter((i) => i !== id)
	} else {
		selectedProcessorIds.value.push(id)
	}
}

function toggleMenu(id: string, event: MouseEvent) {
	event.stopPropagation()
	activeMenuProcessorId.value = activeMenuProcessorId.value === id ? null : id
}

function startNoDataTimer() {
	clearNoDataTimer()
	noDataTimer = setTimeout(() => {
		if (capturedRecords.value.length === 0) {
			stopCapture()
		}
	}, 60000)
}

function clearNoDataTimer() {
	if (noDataTimer) {
		clearTimeout(noDataTimer)
		noDataTimer = undefined
	}
}

function handleWebSocketError(data: any) {
	const title = data?.error?.title ?? 'Failed to capture data'
	const detail =
		data?.error?.detail ??
		(typeof data?.error === 'string' ? data?.error : 'Error starting capture')
	appStore.showError(title, detail)
	stopCapture()
}

function initWebSocket() {
	stopCapture()

	if (!selectedProcessorId.value) return

	const rawBaseUrl = (
		store.clusterUrl ||
		activeClusterURL.value ||
		appHelper.getDefaultClusterValue() ||
		''
	).replace(/\/+$/, '')
	const cleanBase =
		rawBaseUrl.startsWith('/') || rawBaseUrl.startsWith('http') || rawBaseUrl.startsWith('ws')
			? rawBaseUrl
			: `/${rawBaseUrl}`
	const path =
		`${cleanBase}/ws/processors/subscribe_capture/${selectedProcessorId.value}`.replace(
			/^\/\//,
			'/'
		)

	activeConnectionToken++
	const currentToken = activeConnectionToken

	isPooling.value = true
	loading.value = true
	startTimer()

	try {
		ws = new AppWebSocket(path)

		ws.addEventListener('open', () => {
			if (currentToken !== activeConnectionToken) return
			loading.value = true
			if (capturedRecords.value.length === 0) {
				startNoDataTimer()
			}
		})

		ws.addEventListener('message', (e) => {
			if (currentToken !== activeConnectionToken) return
			try {
				const data: any = JSON.parse(e.data)

				if (data?.error || data?.success === false) {
					handleWebSocketError(data)
					return
				}

				const existingIndex = capturedRecords.value.findIndex((v) => v.id === data.index)
				if (existingIndex === -1) {
					const now = new Date()
					const timeStr = now.toTimeString().split(' ')[0]
					capturedRecords.value.push({
						id: data.index ?? capturedRecords.value.length + 1,
						time: timeStr,
						status: '200 OK',
						statusType: 'success',
						message_in: data.message_in,
						message_out: data.message_out
					})
				}

				if (capturedRecords.value.length > 0) {
					clearNoDataTimer()
				}
			} catch (err) {
				console.error('Failed to parse websocket message:', err)
			}
		})

		ws.addEventListener('close', () => {
			if (currentToken !== activeConnectionToken) return
			loading.value = false
			isPooling.value = false
			clearNoDataTimer()
			stopTimer()
		})

		ws.addEventListener('error', () => {
			if (currentToken !== activeConnectionToken) return
			stopCapture()
		})
	} catch (err) {
		if (currentToken === activeConnectionToken) {
			console.error('Failed to initialize WebSocket:', err)
			stopCapture()
		}
	}
}

function startTimer() {
	stopTimer()
	remainingSeconds.value = stopAfterSeconds.value || 10
	timer = setInterval(() => {
		if (remainingSeconds.value > 0) {
			remainingSeconds.value--
		} else {
			stopCapture()
		}
	}, 1000)
}

function stopTimer() {
	if (timer) {
		clearInterval(timer)
		timer = null
	}
}

function stopCapture() {
	if (switchTimeout) {
		clearTimeout(switchTimeout)
		switchTimeout = undefined
	}
	activeConnectionToken++
	stopTimer()
	clearNoDataTimer()
	if (ws) {
		ws.close()
		ws = undefined
	}
	isPooling.value = false
	loading.value = false
}

function restartCapture() {
	capturedRecords.value = []
	selectedRecordIndex.value = 0
	viewMode.value = 'list'
	initWebSocket()
}

function selectProcessor(procId: string) {
	if (selectedProcessorId.value === procId) return
	stopCapture()
	selectedProcessorId.value = procId
	capturedRecords.value = []
	selectedRecordIndex.value = 0
	viewMode.value = 'list'

	if (switchTimeout) clearTimeout(switchTimeout)
	switchTimeout = setTimeout(() => {
		if (isOpen.value && selectedProcessorId.value === procId) {
			initWebSocket()
		}
	}, 150)
}

function selectRecord(idx: number) {
	selectedRecordIndex.value = idx
	viewMode.value = 'detail'
}

function backToList() {
	viewMode.value = 'list'
}

function handleClose() {
	stopCapture()
	activeMenuProcessorId.value = null
	store.closePreviewDataModal()
}

function initializeTargetProcessor() {
	if (modalState.value.processorId) {
		selectedProcessorId.value = modalState.value.processorId
		return
	}
	const firstActive =
		processorsList.value.find((p) => p.status !== false) || processorsList.value[0]
	selectedProcessorId.value = firstActive?.processor_id || ''
}

function handleAddProcessor() {
	if (!nodeId.value) return
	store.openCreateProcessorFormModal(nodeId.value)
}

async function handleEditProcessor(item: IGroupProcessorItem) {
	if (!nodeId.value) return
	activeMenuProcessorId.value = null
	await store.openEditProcessorFormModal(nodeId.value, item)
}

function handleToggleStatus(item: IGroupProcessorItem) {
	activeMenuProcessorId.value = null
	if (nodeId.value) {
		store.toggleProcessorStatus(nodeId.value, item.processor_id)
	}
}

function handleDeleteProcessor(item: IGroupProcessorItem) {
	activeMenuProcessorId.value = null
	if (nodeId.value) {
		store.removeProcessorFromGroup(nodeId.value, item.processor_id)
		selectedProcessorIds.value = selectedProcessorIds.value.filter(
			(id) => id !== item.processor_id
		)
	}
}

function handleBulkDisable() {
	if (nodeId.value && selectedProcessorIds.value.length > 0) {
		store.bulkDisableProcessors(nodeId.value, selectedProcessorIds.value)
		selectedProcessorIds.value = []
	}
}

function handleBulkDelete() {
	if (nodeId.value && selectedProcessorIds.value.length > 0) {
		store.bulkDeleteProcessors(nodeId.value, selectedProcessorIds.value)
		selectedProcessorIds.value = []
	}
}

watch(
	() => isOpen.value,
	(open) => {
		if (open) {
			viewMode.value = 'list'
			initializeTargetProcessor()
			restartCapture()
		} else {
			stopCapture()
		}
	}
)

watch(
	() => activeClusterURL.value,
	() => {
		if (isOpen.value) {
			restartCapture()
		}
	}
)

onMounted(() => {
	if (isOpen.value) {
		viewMode.value = 'list'
		initializeTargetProcessor()
		restartCapture()
	}
})

onUnmounted(() => {
	stopCapture()
})

function getStatusBadgeClass(statusType: string) {
	if (statusType === 'danger')
		return 'bg-rose-100 text-rose-700 dark:bg-rose-950 dark:text-rose-300'
	if (statusType === 'warning')
		return 'bg-amber-100 text-amber-800 dark:bg-amber-950 dark:text-amber-300'
	return 'bg-emerald-100 text-emerald-800 dark:bg-emerald-950 dark:text-emerald-300'
}
</script>

<template>
	<div
		v-if="isOpen"
		class="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-xs p-4 lg:p-8 animate-in fade-in duration-150"
		@click.self="handleClose"
	>
		<div
			class="w-full max-w-[1240px] h-[85vh] bg-white dark:bg-slate-900 rounded-[16px] border border-slate-200 dark:border-slate-800 shadow-2xl flex flex-col overflow-hidden animate-in zoom-in-95 duration-150"
			@click="activeMenuProcessorId = null"
		>
			<!-- Modal Header -->
			<div
				class="flex items-center justify-between px-6 py-3.5 border-b border-slate-200 dark:border-slate-800 bg-white dark:bg-slate-900 shrink-0"
			>
				<!-- Left: Title & Status Badge -->
				<div class="flex items-center gap-3">
					<div class="flex items-center gap-2">
						<svg
							width="18"
							height="18"
							viewBox="0 0 24 24"
							fill="none"
							stroke="#0284C7"
							stroke-width="2"
							stroke-linecap="round"
							stroke-linejoin="round"
						>
							<line x1="18" y1="20" x2="18" y2="10" />
							<line x1="12" y1="20" x2="12" y2="4" />
							<line x1="6" y1="20" x2="6" y2="14" />
						</svg>
						<h3
							class="m-0 !w-auto text-sm font-bold text-slate-900 dark:text-slate-100 whitespace-nowrap shrink-0"
						>
							Preview Data Mode
						</h3>
						<span
							v-if="selectedProcessor"
							:title="selectedProcessor.name"
							class="text-xs px-2 py-0.5 rounded bg-slate-100 dark:bg-slate-800 text-slate-600 dark:text-slate-300 font-medium whitespace-nowrap truncate max-w-[260px] shrink-0"
						>
							{{ selectedProcessor.name }}
						</span>
						<span
							class="text-slate-400 hover:text-slate-600 dark:hover:text-slate-200 cursor-help"
							title="Collects incoming records for a data preview to verify pipeline transformations."
						>
							<svg
								width="14"
								height="14"
								viewBox="0 0 24 24"
								fill="none"
								stroke="currentColor"
								stroke-width="2"
							>
								<circle cx="12" cy="12" r="10" />
								<line x1="12" y1="16" x2="12" y2="12" />
								<line x1="12" y1="8" x2="12.01" y2="8" />
							</svg>
						</span>
					</div>

					<!-- Pooling Status Badge matching Figma -->
					<div
						:class="[
							'px-2.5 py-0.5 rounded-full text-[11px] font-semibold flex items-center gap-1.5 transition-colors',
							isPooling
								? 'bg-sky-50 dark:bg-sky-950/60 text-[#0284C7] dark:text-sky-300 border border-sky-200 dark:border-sky-800'
								: 'bg-emerald-50 dark:bg-emerald-950/60 text-emerald-700 dark:text-emerald-300 border border-emerald-200 dark:border-emerald-800'
						]"
					>
						<span
							:class="[
								'w-2 h-2 rounded-full',
								isPooling ? 'bg-[#0284C7] animate-ping' : 'bg-emerald-500'
							]"
						/>
						<span>{{
							isPooling
								? `Pooling Data (${remainingSeconds}s)`
								: 'Finished Pooling Data'
						}}</span>
					</div>
				</div>

				<!-- Right: Stop After Config & Controls -->
				<div class="flex items-center gap-3">
					<div
						class="flex items-center gap-1.5 text-xs text-slate-600 dark:text-slate-300"
					>
						<span>Stop After</span>
						<input
							v-model.number="stopAfterSeconds"
							type="number"
							min="1"
							max="60"
							:disabled="isPooling"
							class="w-12 px-1.5 py-0.5 text-center text-xs font-semibold rounded border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-800 text-slate-900 dark:text-slate-100 focus:outline-hidden focus:border-[#0284C7] disabled:opacity-50"
						/>
						<span>Seconds</span>
					</div>

					<!-- Stop Button -->
					<button
						type="button"
						class="p-1.5 rounded hover:bg-slate-100 dark:hover:bg-slate-800 text-slate-500 hover:text-slate-700 dark:hover:text-slate-200 transition-colors"
						:title="isPooling ? 'Stop pooling' : 'Start pooling'"
						@click="isPooling ? stopCapture() : restartCapture()"
					>
						<svg
							v-if="isPooling"
							width="15"
							height="15"
							viewBox="0 0 24 24"
							fill="currentColor"
							class="text-rose-500"
						>
							<rect x="5" y="5" width="14" height="14" rx="2" />
						</svg>
						<svg
							v-else
							width="15"
							height="15"
							viewBox="0 0 24 24"
							fill="currentColor"
							class="text-emerald-500"
						>
							<polygon points="5 3 19 12 5 21 5 3" />
						</svg>
					</button>

					<!-- Refresh Button -->
					<button
						type="button"
						class="p-1.5 rounded hover:bg-slate-100 dark:hover:bg-slate-800 text-slate-500 hover:text-slate-700 dark:hover:text-slate-200 transition-colors"
						title="Refresh / Reconnect capture"
						@click="restartCapture"
					>
						<svg
							width="15"
							height="15"
							viewBox="0 0 24 24"
							fill="none"
							stroke="currentColor"
							stroke-width="2"
							:class="loading ? 'animate-spin text-amber-500' : 'text-amber-500'"
						>
							<polyline points="23 4 23 10 17 10" />
							<polyline points="1 20 1 14 7 14" />
							<path
								d="M3.51 9a9 9 0 0 1 14.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0 0 20.49 15"
							/>
						</svg>
					</button>

					<div class="w-px h-5 bg-slate-200 dark:bg-slate-700" />

					<!-- Close Modal -->
					<button
						type="button"
						class="w-7 h-7 flex items-center justify-center rounded text-slate-400 hover:text-slate-600 dark:hover:text-slate-200 hover:bg-slate-100 dark:hover:bg-slate-800 transition-colors"
						title="Close Modal"
						@click="handleClose"
					>
						✕
					</button>
				</div>
			</div>

			<!-- Modal Body: 3-Column Layout -->
			<div class="flex-1 flex overflow-hidden">
				<!-- Column 1: Record Before -->
				<div
					class="w-1/3 flex flex-col border-r border-slate-200 dark:border-slate-800 overflow-hidden bg-white dark:bg-slate-900"
				>
					<div
						class="px-4 py-2.5 bg-slate-50 dark:bg-slate-900/60 border-b border-slate-200 dark:border-slate-800 flex items-center justify-between shrink-0"
					>
						<div class="flex items-center gap-2">
							<span class="text-xs font-bold text-slate-800 dark:text-slate-200">
								Record Before
							</span>
							<span
								class="px-1.5 py-0.2 rounded text-[10px] bg-slate-200 dark:bg-slate-800 text-slate-600 dark:text-slate-300 font-semibold"
							>
								{{ capturedRecords.length }}
							</span>
						</div>

						<span
							v-if="viewMode === 'detail'"
							class="text-[11px] font-semibold text-[#0284C7] cursor-pointer hover:underline"
							@click="backToList"
						>
							View List
						</span>
					</div>

					<!-- Column 1 Content -->
					<div class="flex-1 overflow-y-auto p-3 flex flex-col gap-2">
						<!-- 1. Loading State -->
						<div
							v-if="isPooling && capturedRecords.length === 0"
							class="h-full flex flex-col items-center justify-center p-6 text-center text-slate-400"
						>
							<div
								class="w-8 h-8 rounded-full border-2 border-sky-500 border-t-transparent animate-spin mb-3"
							/>
							<div class="text-xs font-semibold text-slate-700 dark:text-slate-200">
								Listening for incoming WebSocket records...
							</div>
							<div class="text-[11px] text-slate-400 mt-1">
								Waiting for data flow through processor
							</div>
						</div>

						<!-- 2. Data Not Found State matching Figma -->
						<div
							v-else-if="!isPooling && capturedRecords.length === 0"
							class="h-full flex flex-col items-center justify-center p-6 text-center"
						>
							<div
								class="w-12 h-12 rounded-full bg-amber-50 dark:bg-amber-950/60 border border-amber-200 dark:border-amber-800 flex items-center justify-center text-amber-500 mb-3 shadow-xs"
							>
								<svg
									width="24"
									height="24"
									viewBox="0 0 24 24"
									fill="none"
									stroke="currentColor"
									stroke-width="2"
								>
									<circle cx="12" cy="12" r="10" />
									<line x1="12" y1="8" x2="12" y2="12" />
									<line x1="12" y1="16" x2="12.01" y2="16" />
								</svg>
							</div>
							<div class="text-sm font-semibold text-slate-800 dark:text-slate-100">
								Sorry, Data not found.
							</div>
							<div
								class="text-[11px] text-slate-500 dark:text-slate-400 mt-1 max-w-[220px]"
							>
								Check your configuration filters or wait for fresh polling entries.
							</div>
						</div>

						<!-- 3. List View Mode matching Figma #2778:140500 -->
						<template v-else-if="viewMode === 'list'">
							<div
								v-for="(rec, idx) in capturedRecords"
								:key="rec.id"
								:class="[
									'p-3 rounded-lg border cursor-pointer transition-all flex flex-col gap-1.5 shadow-xs',
									selectedRecordIndex === idx
										? 'border-[#0284C7] bg-sky-50/40 dark:bg-sky-950/30'
										: 'border-slate-200 dark:border-slate-800 bg-white dark:bg-slate-800/40 hover:border-sky-300'
								]"
								@click="selectRecord(idx)"
							>
								<div class="flex items-center justify-between">
									<span class="text-[10px] text-slate-400 font-mono">
										[{{ rec.time }}]
									</span>
									<span
										:class="[
											'text-[9px] font-bold px-1.5 py-0.5 rounded tracking-wider',
											getStatusBadgeClass(rec.statusType)
										]"
									>
										{{ rec.status }}
									</span>
								</div>

								<div class="text-xs font-bold text-slate-900 dark:text-slate-100">
									Record #{{ rec.id }}
								</div>

								<div
									class="text-[10px] font-mono text-slate-500 dark:text-slate-400 truncate"
								>
									{{ JSON.stringify(rec.message_in) }}
								</div>
							</div>
						</template>

						<!-- 4. Detail View Mode matching Figma #2778:140755 -->
						<div
							v-else
							class="h-full bg-slate-50 dark:bg-slate-950 rounded-lg p-3 text-xs font-mono text-[#0284C7] dark:text-sky-300 overflow-auto border border-slate-200 dark:border-slate-800"
						>
							<pre v-if="activeRecord">{{
								JSON.stringify(activeRecord.message_in, null, 2)
							}}</pre>
						</div>
					</div>
				</div>

				<!-- Column 2: Processors in Sequence matching Figma -->
				<div
					class="w-1/3 flex flex-col border-r border-slate-200 dark:border-slate-800 overflow-hidden bg-slate-50/30 dark:bg-slate-950/20"
				>
					<div
						class="px-4 py-2.5 bg-slate-50 dark:bg-slate-900/60 border-b border-slate-200 dark:border-slate-800 flex items-center justify-between shrink-0"
					>
						<span class="text-xs font-bold text-slate-800 dark:text-slate-200">
							Processors in Sequence
						</span>
						<span class="text-[11px] text-slate-400"> Pipeline Flow </span>
					</div>

					<div class="flex-1 overflow-y-auto p-4 flex flex-col gap-3">
						<!-- Back to List button in detail view matching Figma #2778:140547 -->
						<div
							v-if="viewMode === 'detail'"
							class="flex items-center justify-between pb-2 border-b border-slate-200 dark:border-slate-800"
						>
							<button
								type="button"
								class="flex items-center gap-1.5 text-xs font-semibold text-[#0284C7] hover:underline cursor-pointer"
								@click="backToList"
							>
								<svg
									width="14"
									height="14"
									viewBox="0 0 24 24"
									fill="none"
									stroke="currentColor"
									stroke-width="2.5"
								>
									<polyline points="15 18 9 12 15 6" />
								</svg>
								Back to List
							</button>
							<span class="text-[10px] text-slate-400 font-mono">
								Viewing Record #{{ activeRecord?.id }}
							</span>
						</div>

						<!-- Add Processor Button matching Figma #2778:123243 -->
						<button
							type="button"
							class="w-full py-2 px-3 rounded-lg bg-primary hover:bg-primary/90 text-primary-foreground text-xs font-semibold flex items-center justify-center gap-1.5 transition-colors shadow-xs cursor-pointer"
							@click="handleAddProcessor"
						>
							<svg
								width="14"
								height="14"
								viewBox="0 0 24 24"
								fill="none"
								stroke="currentColor"
								stroke-width="2.5"
							>
								<line x1="12" y1="5" x2="12" y2="19" />
								<line x1="5" y1="12" x2="19" y2="12" />
							</svg>
							Add Processor
						</button>

						<!-- Bulk Action Toolbar matching Figma #2778:123244 -->
						<div
							class="flex items-center justify-between text-xs py-1 border-b border-slate-200 dark:border-slate-800"
						>
							<label class="flex items-center gap-2 cursor-pointer select-none">
								<input
									type="checkbox"
									:checked="allProcessorsSelected"
									class="rounded border-slate-300 text-sky-600 focus:ring-sky-500"
									@change="
										toggleSelectAll(($event.target as HTMLInputElement).checked)
									"
								/>
								<span
									class="text-xs text-slate-600 dark:text-slate-300 font-medium"
								>
									Select All Processor
								</span>
							</label>

							<div class="flex items-center gap-1">
								<button
									type="button"
									:disabled="selectedProcessorIds.length === 0"
									class="px-2 py-0.5 rounded text-[11px] font-medium text-slate-600 dark:text-slate-300 hover:bg-slate-200 dark:hover:bg-slate-800 disabled:opacity-40 transition-colors"
									@click="handleBulkDisable"
								>
									Disable Selected
								</button>
								<button
									type="button"
									:disabled="selectedProcessorIds.length === 0"
									class="px-2 py-0.5 rounded text-[11px] font-medium text-rose-600 hover:bg-rose-50 dark:hover:bg-rose-950/50 disabled:opacity-40 transition-colors"
									@click="handleBulkDelete"
								>
									Delete Selected
								</button>
							</div>
						</div>

						<!-- Processor Sequence List with Connectors -->
						<div class="flex flex-col items-center gap-2">
							<template v-if="processorsList.length === 0">
								<div class="text-xs text-slate-400 py-8 text-center">
									No processors found in group
								</div>
							</template>

							<template
								v-else
								v-for="(proc, pIdx) in processorsList"
								:key="proc.processor_id"
							>
								<!-- Processor Row Card matching Figma #2778:123249 -->
								<div
									:class="[
										'relative w-full p-2.5 rounded-lg border bg-white dark:bg-slate-900 shadow-xs flex items-center justify-between gap-2 transition-all cursor-pointer',
										proc.status === false
											? 'opacity-60 border-slate-200 dark:border-slate-800'
											: 'border-slate-200 dark:border-slate-700 hover:border-sky-300 dark:hover:border-sky-700',
										selectedProcessorId === proc.processor_id
											? 'ring-2 ring-sky-500 border-sky-500 bg-sky-50/40 dark:bg-sky-950/40'
											: ''
									]"
									@click="selectProcessor(proc.processor_id)"
								>
									<div class="flex items-center gap-2 min-w-0">
										<!-- Drag handle icon -->
										<div class="text-slate-400 cursor-grab">
											<svg
												width="12"
												height="12"
												viewBox="0 0 24 24"
												fill="currentColor"
											>
												<circle cx="9" cy="5" r="1.5" />
												<circle cx="15" cy="5" r="1.5" />
												<circle cx="9" cy="12" r="1.5" />
												<circle cx="15" cy="12" r="1.5" />
											</svg>
										</div>

										<!-- Selection Checkbox -->
										<input
											type="checkbox"
											:checked="
												selectedProcessorIds.includes(proc.processor_id)
											"
											class="rounded border-slate-300 text-sky-600 focus:ring-sky-500 cursor-pointer"
											@click.stop="
												toggleSelectProcessor(proc.processor_id, $event)
											"
										/>

										<!-- Processor Name -->
										<span
											class="text-xs font-semibold text-slate-800 dark:text-slate-100 truncate"
										>
											{{ proc.name }}
										</span>

										<!-- Target Capture Badge -->
										<span
											v-if="selectedProcessorId === proc.processor_id"
											class="px-1.5 py-0.5 rounded text-[9px] font-bold bg-[#0284C7] text-white shrink-0"
										>
											Target
										</span>
									</div>

									<div class="flex items-center gap-2 shrink-0">
										<!-- Module Badge -->
										<span
											:class="[
												'text-[9px] font-bold px-1.5 py-0.5 rounded uppercase tracking-wider',
												getProcessorModuleBadgeClass(proc.module, false)
											]"
										>
											{{ (proc.module || 'null').replace(/_/g, ' ') }}
										</span>

										<!-- 3-Dots Action Menu matching Figma -->
										<button
											type="button"
											class="p-1 rounded text-slate-400 hover:text-slate-600 dark:hover:text-slate-200 hover:bg-slate-100 dark:hover:bg-slate-800"
											title="More Actions"
											@click="toggleMenu(proc.processor_id, $event)"
										>
											<svg
												width="14"
												height="14"
												viewBox="0 0 24 24"
												fill="currentColor"
											>
												<circle cx="12" cy="5" r="1.75" />
												<circle cx="12" cy="12" r="1.75" />
												<circle cx="12" cy="19" r="1.75" />
											</svg>
										</button>
									</div>

									<!-- Action Menu Dropdown -->
									<div
										v-if="activeMenuProcessorId === proc.processor_id"
										class="absolute right-2 top-9 z-30 w-36 bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-lg shadow-xl py-1 text-xs animate-in fade-in zoom-in-95 duration-100"
										@click.stop
									>
										<button
											type="button"
											class="w-full px-3 py-1.5 text-left text-slate-700 dark:text-slate-200 hover:bg-slate-100 dark:hover:bg-slate-800 flex items-center gap-2"
											@click="handleEditProcessor(proc)"
										>
											View / Edit
										</button>
										<button
											type="button"
											class="w-full px-3 py-1.5 text-left text-slate-700 dark:text-slate-200 hover:bg-slate-100 dark:hover:bg-slate-800 flex items-center gap-2"
											@click="handleToggleStatus(proc)"
										>
											{{ proc.status === false ? 'Enable' : 'Disable' }}
										</button>
										<div class="h-px bg-slate-100 dark:bg-slate-800 my-1" />
										<button
											type="button"
											class="w-full px-3 py-1.5 text-left text-rose-600 hover:bg-rose-50 dark:hover:bg-rose-950/50 flex items-center gap-2"
											@click="handleDeleteProcessor(proc)"
										>
											Delete
										</button>
									</div>
								</div>

								<!-- Connector Arrow Down matching Figma #2778:123255 -->
								<div
									v-if="pIdx < processorsList.length - 1"
									class="text-[#0284C7] py-0.5"
								>
									<svg
										width="14"
										height="14"
										viewBox="0 0 24 24"
										fill="none"
										stroke="currentColor"
										stroke-width="2.5"
										stroke-linecap="round"
										stroke-linejoin="round"
									>
										<line x1="12" y1="5" x2="12" y2="19" />
										<polyline points="19 12 12 19 5 12" />
									</svg>
								</div>
							</template>
						</div>
					</div>
				</div>

				<!-- Column 3: Record After matching Figma -->
				<div class="w-1/3 flex flex-col overflow-hidden bg-white dark:bg-slate-900">
					<div
						class="px-4 py-2.5 bg-slate-50 dark:bg-slate-900/60 border-b border-slate-200 dark:border-slate-800 flex items-center justify-between shrink-0"
					>
						<span class="text-xs font-bold text-slate-800 dark:text-slate-200">
							Record After
						</span>
						<span
							class="text-[10px] text-emerald-600 dark:text-emerald-400 font-semibold flex items-center gap-1"
						>
							<span class="w-1.5 h-1.5 rounded-full bg-emerald-500" />
							Transformed Payload
						</span>
					</div>

					<div class="flex-1 overflow-y-auto p-3 flex flex-col gap-2">
						<!-- 1. Loading State -->
						<div
							v-if="isPooling && capturedRecords.length === 0"
							class="h-full flex flex-col items-center justify-center p-6 text-center text-slate-400"
						>
							<div
								class="w-8 h-8 rounded-full border-2 border-sky-500 border-t-transparent animate-spin mb-3"
							/>
							<div class="text-xs font-semibold text-slate-700 dark:text-slate-200">
								Listening for incoming WebSocket records...
							</div>
							<div class="text-[11px] text-slate-400 mt-1">
								Waiting for transformed payload
							</div>
						</div>

						<!-- 2. Data Not Found State matching Figma -->
						<div
							v-else-if="!isPooling && capturedRecords.length === 0"
							class="h-full flex flex-col items-center justify-center p-6 text-center"
						>
							<div
								class="w-12 h-12 rounded-full bg-amber-50 dark:bg-amber-950/60 border border-amber-200 dark:border-amber-800 flex items-center justify-center text-amber-500 mb-3 shadow-xs"
							>
								<svg
									width="24"
									height="24"
									viewBox="0 0 24 24"
									fill="none"
									stroke="currentColor"
									stroke-width="2"
								>
									<circle cx="12" cy="12" r="10" />
									<line x1="12" y1="8" x2="12" y2="12" />
									<line x1="12" y1="16" x2="12.01" y2="16" />
								</svg>
							</div>
							<div class="text-sm font-semibold text-slate-800 dark:text-slate-100">
								Sorry, Data not found.
							</div>
							<div
								class="text-[11px] text-slate-500 dark:text-slate-400 mt-1 max-w-[220px]"
							>
								Check your configuration filters or wait for fresh polling entries.
							</div>
						</div>

						<!-- 3. List View mode waiting placeholder -->
						<div
							v-else-if="viewMode === 'list'"
							class="h-full flex flex-col items-center justify-center p-6 text-center text-slate-400"
						>
							<svg
								width="32"
								height="32"
								viewBox="0 0 24 24"
								fill="none"
								stroke="currentColor"
								stroke-width="1.5"
								class="text-slate-300 dark:text-slate-600 mb-2"
							>
								<path
									d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"
								/>
								<polyline points="14 2 14 8 20 8" />
								<line x1="16" y1="13" x2="8" y2="13" />
								<line x1="16" y1="17" x2="8" y2="17" />
								<polyline points="10 9 9 9 8 9" />
							</svg>
							<span class="text-xs font-medium text-slate-500">
								Select a record from the list
							</span>
							<span class="text-[11px] text-slate-400 mt-0.5">
								Click any record in Record Before to view full transformation
							</span>
						</div>

						<!-- 4. Detail View Mode matching Figma #2778:140814 -->
						<div
							v-else
							class="h-full bg-slate-50 dark:bg-slate-950 rounded-lg p-3 text-xs font-mono text-[#0284C7] dark:text-sky-300 overflow-auto border border-slate-200 dark:border-slate-800"
						>
							<div
								class="text-[11px] text-slate-400 pb-2 border-b border-slate-200 dark:border-slate-800 mb-2 flex items-center justify-between"
							>
								<span>Result for Record #{{ activeRecord?.id }}</span>
								<span class="text-[10px] text-slate-500">Transformed Output</span>
							</div>
							<pre v-if="activeRecord && activeRecord.message_out !== undefined">{{
								JSON.stringify(activeRecord.message_out, null, 2)
							}}</pre>
							<div v-else class="text-slate-400 italic text-center py-6">
								No transformation output produced
							</div>
						</div>
					</div>
				</div>
			</div>
		</div>
	</div>
</template>

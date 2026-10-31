<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed, ref } from 'vue'
import { onClickOutside } from '@vueuse/core'
import {
	getProcessorModuleBadgeClass,
	usePipelineBuildStore
} from '@/stores/pipeline.builder.store'
import type { IGroupProcessorItem } from '@/stores/pipeline.builder.store'
import { useProcessorStore } from '@/stores/processor.store'
import AppInputText from '@/components/app/input/AppInputText.vue'
import ModalProcessorForm from '@/components/configuration/processor/ModalForm.vue'

const store = usePipelineBuildStore()
const processorStore = useProcessorStore()

const isOpen = computed(() => {
	const node = store.selectedNode as any
	return !!node && node.type === 'processor'
})

const currentNode = computed(() => store.selectedNode as any)
const nodeId = computed(() => currentNode.value?.id)

const searchQuery = ref('')
const selectedProcessorIds = ref<string[]>([])
const activeMenuProcessorId = ref<string | null>(null)
const menuRef = ref<HTMLElement | null>(null)

// Drag-and-drop state
const draggedIndex = ref<number | null>(null)
const dragOverIndex = ref<number | null>(null)

onClickOutside(menuRef, () => {
	activeMenuProcessorId.value = null
})

const processorsList = computed<IGroupProcessorItem[]>(() => {
	return currentNode.value?.data?.processors || []
})

const filteredProcessors = computed(() => {
	let list = processorsList.value
	if (searchQuery.value.trim()) {
		const q = searchQuery.value.toLowerCase()
		list = list.filter((p) => {
			const name = (p.name || '').toLowerCase()
			const module = (p.module || '').toLowerCase()
			return name.includes(q) || module.includes(q)
		})
	}
	return list
})

const activeCount = computed(() => {
	return processorsList.value.filter((p) => p.status !== false).length
})

const isAllSelected = computed(() => {
	if (processorsList.value.length === 0) return false
	return (
		processorsList.value.length > 0 &&
		processorsList.value.every((p) => selectedProcessorIds.value.includes(p.processor_id))
	)
})

function toggleSelectAll() {
	if (processorsList.value.length === 0) return
	if (isAllSelected.value) {
		selectedProcessorIds.value = []
	} else {
		selectedProcessorIds.value = processorsList.value.map((p) => p.processor_id)
	}
}

function toggleSelect(id: string) {
	const idx = selectedProcessorIds.value.indexOf(id)
	if (idx === -1) {
		selectedProcessorIds.value.push(id)
	} else {
		selectedProcessorIds.value.splice(idx, 1)
	}
}

function closeDrawer() {
	store.selectedNode = undefined
}

function handleOpenAddProcessor() {
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

function handleOpenPreviewData() {
	if (nodeId.value) {
		store.openPreviewDataModal(nodeId.value)
	}
}

function handlePreviewProcessor(item: IGroupProcessorItem) {
	activeMenuProcessorId.value = null
	if (nodeId.value) {
		store.openPreviewDataModal(nodeId.value, item.processor_id)
	}
}

function toggleMenu(id: string, event: MouseEvent) {
	event.stopPropagation()
	if (activeMenuProcessorId.value === id) {
		activeMenuProcessorId.value = null
	} else {
		activeMenuProcessorId.value = id
	}
}

// Drag & Drop handlers
function onDragStart(index: number, event: DragEvent) {
	draggedIndex.value = index
	if (event.dataTransfer) {
		event.dataTransfer.effectAllowed = 'move'
		event.dataTransfer.setData('text/plain', String(index))
	}
}

function onDragOver(index: number, event: DragEvent) {
	event.preventDefault()
	if (event.dataTransfer) {
		event.dataTransfer.dropEffect = 'move'
	}
	dragOverIndex.value = index
}

function onDrop(index: number, event: DragEvent) {
	event.preventDefault()
	if (draggedIndex.value !== null && draggedIndex.value !== index && nodeId.value) {
		store.reorderProcessorGroup(nodeId.value, draggedIndex.value, index)
	}
	draggedIndex.value = null
	dragOverIndex.value = null
}

function onDragEnd() {
	draggedIndex.value = null
	dragOverIndex.value = null
}
</script>

<template>
	<!-- Side Drawer matching Figma #1790:89305 -->
	<aside
		v-if="isOpen"
		class="absolute top-[66px] right-0 bottom-0 w-[420px] bg-white dark:bg-slate-900 border-l border-t border-slate-200 dark:border-slate-800 shadow-2xl z-30 flex flex-col animate-in slide-in-from-right duration-200 select-none nokey nowheel nodrag nopan"
	>
		<!-- Drawer Header matching Figma #1790:89306 -->
		<div
			class="flex items-center justify-between px-5 py-3.5 border-b border-slate-200 dark:border-slate-800 gap-2"
		>
			<div class="flex flex-col min-w-0">
				<h2 class="text-base font-bold text-slate-900 dark:text-slate-100 truncate">
					{{ currentNode?.data?.name || currentNode?.label || 'Processor Group' }}
				</h2>
				<span class="text-xs text-slate-500 dark:text-slate-400">
					{{ activeCount }} active processors in sequence
				</span>
			</div>

			<div class="flex items-center gap-2 shrink-0">
				<!-- Preview Data Button matching Figma #2060:129460 -->
				<UiButton
					size="xs"
					class="h-7 text-xs font-semibold gap-1"
					@click="handleOpenPreviewData"
				>
					<UiIcon name="eye" size="14px" />
					<span>Preview Data</span>
				</UiButton>

				<!-- Close Button matching Figma #1801:107002 -->
				<UiButton
					variant="ghost"
					size="icon"
					class="h-7 w-7 text-muted-foreground hover:text-foreground"
					title="Close Drawer"
					@click="closeDrawer"
				>
					<UiIcon name="close" size="14px" />
				</UiButton>
			</div>
		</div>

		<!-- Search Bar & Add Button matching Figma #1790:89313 -->
		<div class="px-5 pt-3.5 pb-2 flex items-center gap-2">
			<div class="flex-1">
				<AppInputText
					v-model="searchQuery"
					placeholder="Search processor inside group..."
					pre-icon="magnify"
					variant="outline"
				/>
			</div>

			<UiButton
				size="xs"
				class="h-8 text-xs font-medium gap-1 shrink-0"
				@click="handleOpenAddProcessor"
			>
				<UiIcon name="plus" size="14px" />
				<span>Add Processor</span>
			</UiButton>
		</div>

		<!-- Bulk Action Toolbar matching Figma #1980:19284 -->
		<div
			class="px-5 py-2 flex items-center justify-between border-b border-slate-100 dark:border-slate-800/80 text-xs"
		>
			<div
				class="flex items-center gap-2 cursor-pointer select-none group"
				@click="toggleSelectAll"
			>
				<UiCheckbox
					:checked="
						isAllSelected
							? true
							: selectedProcessorIds.length > 0
								? 'indeterminate'
								: false
					"
					class="pointer-events-none"
				/>
				<span
					class="font-medium text-slate-700 dark:text-slate-300 group-hover:text-slate-900 dark:group-hover:text-slate-100 transition-colors"
				>
					Select All Processor
				</span>
			</div>

			<div class="flex items-center gap-2">
				<UiButton
					variant="ghost"
					size="xs"
					:disabled="selectedProcessorIds.length === 0"
					class="h-6 text-xs px-2"
					@click="handleBulkDisable"
				>
					Disable Selected
				</UiButton>
				<UiButton
					variant="ghost"
					size="xs"
					:disabled="selectedProcessorIds.length === 0"
					class="h-6 text-xs px-2 text-destructive hover:text-destructive hover:bg-destructive/10"
					@click="handleBulkDelete"
				>
					Delete Selected
				</UiButton>
			</div>
		</div>

		<!-- Draggable Processors List matching Figma #1790:89318 -->
		<div class="flex-1 overflow-y-auto px-4 py-3 flex flex-col gap-1.5">
			<!-- Empty State -->
			<div
				v-if="filteredProcessors.length === 0"
				class="flex flex-col items-center justify-center py-12 text-center text-slate-400"
			>
				<svg
					width="32"
					height="32"
					viewBox="0 0 24 24"
					fill="none"
					stroke="currentColor"
					stroke-width="1.5"
					class="mb-2 text-slate-300 dark:text-slate-600"
				>
					<rect x="2" y="2" width="20" height="8" rx="2" />
					<rect x="2" y="14" width="20" height="8" rx="2" />
					<line x1="6" y1="6" x2="6.01" y2="6" />
					<line x1="6" y1="18" x2="6.01" y2="18" />
				</svg>
				<p class="text-xs font-medium text-slate-600 dark:text-slate-400">
					No processors found
				</p>
				<p class="text-[11px] text-slate-400 mt-0.5">
					{{
						searchQuery
							? 'Try adjusting your search query'
							: 'Add your first processor to this sequence'
					}}
				</p>
				<button
					v-if="!searchQuery"
					type="button"
					class="mt-3 px-3 py-1.5 text-xs rounded bg-primary hover:bg-primary/90 text-primary-foreground font-medium transition-colors"
					@click="handleOpenAddProcessor"
				>
					+ Add Processor
				</button>
			</div>

			<!-- Processor Rows matching Figma #1790:89319 -->
			<div
				v-for="(item, index) in filteredProcessors"
				:key="item.processor_id"
				draggable="true"
				:class="[
					'group flex items-center justify-between p-2 rounded-lg border transition-all',
					dragOverIndex === index
						? 'border-sky-500 bg-sky-50/50 dark:bg-sky-950/30'
						: item.status === false
							? 'border-slate-200 dark:border-slate-800 bg-slate-50/80 dark:bg-slate-900/40'
							: 'border-slate-200 dark:border-slate-800 bg-white dark:bg-slate-800/60 hover:border-slate-300 dark:hover:border-slate-700',
					item.status === false ? 'opacity-60' : 'opacity-100'
				]"
				@dragstart="onDragStart(index, $event)"
				@dragover="onDragOver(index, $event)"
				@drop="onDrop(index, $event)"
				@dragend="onDragEnd"
			>
				<div class="flex items-center gap-2 min-w-0 flex-1">
					<!-- Drag Handle Icon matching Figma #1852:16539 -->
					<div
						class="cursor-grab active:cursor-grabbing text-slate-400 hover:text-slate-600 dark:hover:text-slate-200 p-0.5"
						title="Drag to reorder"
					>
						<svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor">
							<circle cx="9" cy="5" r="1.5" />
							<circle cx="15" cy="5" r="1.5" />
							<circle cx="9" cy="12" r="1.5" />
							<circle cx="15" cy="12" r="1.5" />
							<circle cx="9" cy="19" r="1.5" />
							<circle cx="15" cy="19" r="1.5" />
						</svg>
					</div>

					<div class="cursor-pointer" @click.stop="toggleSelect(item.processor_id)">
						<UiCheckbox
							:checked="selectedProcessorIds.includes(item.processor_id)"
							class="pointer-events-none"
						/>
					</div>

					<!-- Processor Name -->
					<span
						:class="[
							'text-xs font-medium truncate flex-1 min-w-0 transition-colors',
							item.status === false
								? 'text-slate-400 dark:text-slate-500 line-through'
								: 'text-slate-800 dark:text-slate-100'
						]"
						:title="item.name"
					>
						{{ item.name }}
					</span>

					<!-- Disabled Status Badge -->
					<span
						v-if="item.status === false"
						class="text-[9px] font-bold px-1.5 py-0.5 rounded border border-slate-200 dark:border-slate-700 bg-slate-100 dark:bg-slate-800 text-slate-500 dark:text-slate-400 uppercase tracking-wider shrink-0"
					>
						Disabled
					</span>

					<!-- Module Badge matching Figma #1790:89324 -->
					<span
						:class="[
							'text-[10px] font-bold px-1.5 py-0.5 rounded border uppercase tracking-wider shrink-0',
							getProcessorModuleBadgeClass(item.module)
						]"
					>
						{{ (item.module || 'null').replace(/_/g, ' ') }}
					</span>
				</div>

				<!-- 3-Dots Action Menu matching Figma #2065:131285 -->
				<div
					class="relative shrink-0 ml-1.5"
					:ref="
						(el) => {
							if (activeMenuProcessorId === item.processor_id)
								menuRef = el as HTMLElement
						}
					"
				>
					<button
						type="button"
						class="w-6 h-6 flex items-center justify-center rounded text-slate-400 hover:text-slate-600 dark:hover:text-slate-200 hover:bg-slate-100 dark:hover:bg-slate-700 transition-colors"
						title="Actions"
						@click="toggleMenu(item.processor_id, $event)"
					>
						<svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor">
							<circle cx="12" cy="5" r="2" />
							<circle cx="12" cy="12" r="2" />
							<circle cx="12" cy="19" r="2" />
						</svg>
					</button>

					<!-- Floating Dropdown Menu matching Figma #2065:131108 -->
					<div
						v-if="activeMenuProcessorId === item.processor_id"
						class="absolute right-0 top-full mt-1 w-36 bg-white dark:bg-slate-800 rounded-lg shadow-xl border border-slate-200 dark:border-slate-700 py-1.5 z-50 animate-in fade-in zoom-in-95 duration-100 text-xs font-medium"
					>
						<!-- Preview Data -->
						<button
							type="button"
							class="w-full px-3.5 py-2 text-slate-800 dark:text-slate-100 hover:bg-slate-100 dark:hover:bg-slate-700/60 text-left transition-colors"
							@click="handlePreviewProcessor(item)"
						>
							Preview Data
						</button>

						<!-- View / Edit -->
						<button
							type="button"
							class="w-full px-3.5 py-2 text-slate-800 dark:text-slate-100 hover:bg-slate-100 dark:hover:bg-slate-700/60 text-left transition-colors"
							@click="handleEditProcessor(item)"
						>
							View / Edit
						</button>

						<!-- Disable / Enable -->
						<button
							type="button"
							class="w-full px-3.5 py-2 text-slate-800 dark:text-slate-100 hover:bg-slate-100 dark:hover:bg-slate-700/60 text-left transition-colors"
							@click="handleToggleStatus(item)"
						>
							{{ item.status === false ? 'Enable' : 'Disable' }}
						</button>

						<!-- Delete -->
						<button
							type="button"
							class="w-full px-3.5 py-2 text-red-600 dark:text-red-400 hover:bg-red-50 dark:hover:bg-red-950/40 text-left transition-colors"
							@click="handleDeleteProcessor(item)"
						>
							Delete
						</button>
					</div>
				</div>
			</div>
		</div>
	</aside>

	<!-- Processor Configuration Modal for View/Edit & Add -->
	<ModalProcessorForm v-if="processorStore.processorModalShow" />
</template>

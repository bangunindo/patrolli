<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { usePipelineBuildStore } from '@/stores/pipeline.builder.store'
import AppInputText from '@/components/app/input/AppInputText.vue'
import AppInputSelect from '@/components/app/input/AppInputSelect.vue'

const store = usePipelineBuildStore()

const searchQuery = ref('')
const selectedHostFilter = ref('all')
const selectedItemIds = ref<string[]>([])

function extractItemIds(items: any[]): string[] {
	if (!Array.isArray(items)) return []
	return items
		.map((i: any) => {
			const id = typeof i === 'string' ? i : i?.item_id || i?.id
			return typeof id === 'string' ? id.trim() : ''
		})
		.filter((id): id is string => id.length > 0)
}

function getItemId(item: any): string {
	return String(item?.item_id || item?.id || '').trim()
}

function isItemChecked(item: any): boolean {
	const id = getItemId(item).toLowerCase()
	return selectedItemIds.value.some((selId) => selId.toLowerCase() === id)
}

// Populate pre-selected items when modal opens
watch(
	() => [store.edgeItemModal.isOpen, store.edgeItemModal.edgeId],
	([isOpen, edgeId]) => {
		if (isOpen && edgeId) {
			searchQuery.value = ''
			selectedHostFilter.value = 'all'
			const edge = store.edges.find((e: any) => e.id === edgeId)
			const ids = extractItemIds(edge?.data?.selectedItemIds)
			selectedItemIds.value = ids
		}
	},
	{ immediate: true }
)

const availableHosts = computed(() => {
	const hostIds = store.pipelineDetail?.host_ids || []
	return store.availableHosts.filter((h) => hostIds.includes(h.host_id))
})

const hostFilterOptions = computed(() => [
	{ label: 'All Pipeline Hosts', value: 'all' },
	...availableHosts.value.map((h) => ({
		label: h.name || h.host_id,
		value: h.host_id
	}))
])

const filteredItems = computed(() => {
	let list = store.pipelineItems || []
	if (selectedHostFilter.value !== 'all') {
		list = list.filter((i) => i.host_id === selectedHostFilter.value)
	}
	if (searchQuery.value.trim()) {
		const q = searchQuery.value.toLowerCase()
		list = list.filter((i) => {
			const name = (i.name || i.resolved_name || '').toLowerCase()
			const hName = (i.host_name || '').toLowerCase()
			return name.includes(q) || hName.includes(q)
		})
	}
	return list
})

const isAllSelected = computed(() => {
	if (filteredItems.value.length === 0) return false
	return filteredItems.value.every((i) => isItemChecked(i))
})

function toggleSelectAll() {
	if (filteredItems.value.length === 0) return
	if (isAllSelected.value) {
		const currentIds = new Set(filteredItems.value.map((i) => getItemId(i).toLowerCase()))
		selectedItemIds.value = selectedItemIds.value.filter(
			(id) => !currentIds.has(id.toLowerCase())
		)
	} else {
		const set = new Set(selectedItemIds.value)
		filteredItems.value.forEach((i) => {
			const id = getItemId(i)
			if (id) set.add(id)
		})
		selectedItemIds.value = Array.from(set)
	}
}

function toggleItem(item: any) {
	const itemId = getItemId(item)
	if (!itemId) return
	const lower = itemId.toLowerCase()
	const idx = selectedItemIds.value.findIndex((id) => id.toLowerCase() === lower)
	if (idx > -1) {
		selectedItemIds.value.splice(idx, 1)
	} else {
		selectedItemIds.value.push(itemId)
	}
}

function handleClose() {
	store.closeEdgeItemSelector()
}

function handleApply() {
	if (store.edgeItemModal.edgeId) {
		store.applyEdgeItems(store.edgeItemModal.edgeId, selectedItemIds.value)
	}
}
</script>

<template>
	<div
		v-if="store.edgeItemModal.isOpen"
		class="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-xs p-4 animate-in fade-in duration-150"
	>
		<div
			class="w-full max-w-2xl bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-xl shadow-2xl flex flex-col max-h-[85vh] overflow-hidden"
			@click.stop
		>
			<!-- Header -->
			<div
				class="flex items-center justify-between px-6 py-4 border-b border-slate-200 dark:border-slate-800"
			>
				<div>
					<h3
						class="text-base font-bold text-slate-900 dark:text-slate-100 flex items-center gap-2"
					>
						<svg
							width="18"
							height="18"
							viewBox="0 0 24 24"
							fill="none"
							stroke="#0284C7"
							stroke-width="2.5"
							stroke-linecap="round"
							stroke-linejoin="round"
						>
							<polygon points="12 2 2 7 12 12 22 7 12 2" />
							<polyline points="2 17 12 22 22 17" />
							<polyline points="2 12 12 17 22 12" />
						</svg>
						Select Items for Connection
					</h3>
					<p class="text-xs text-slate-500 dark:text-slate-400 mt-0.5">
						Choose items from selected pipeline hosts to process through this stream
					</p>
				</div>
				<UiButton
					variant="ghost"
					size="icon"
					class="h-8 w-8 text-muted-foreground hover:text-foreground"
					@click="handleClose"
				>
					<UiIcon name="close" size="16px" />
				</UiButton>
			</div>

			<!-- Search & Host Filter Toolbar -->
			<div
				class="px-6 py-3 border-b border-slate-100 dark:border-slate-800/80 bg-slate-50/50 dark:bg-slate-950/30 flex items-center gap-3"
			>
				<!-- Search -->
				<div class="flex-1">
					<AppInputText
						v-model="searchQuery"
						placeholder="Search items by name..."
						pre-icon="magnify"
						variant="outline"
					/>
				</div>

				<!-- Host Filter (if > 1 host) -->
				<div v-if="availableHosts.length > 1" class="w-52 shrink-0">
					<AppInputSelect
						v-model="selectedHostFilter"
						:options="hostFilterOptions"
						label-key="label"
						value-key="value"
						placeholder="All Pipeline Hosts"
					/>
				</div>
			</div>

			<!-- Selection Stats Bar -->
			<div
				class="px-6 py-2 bg-slate-100/60 dark:bg-slate-800/40 border-b border-slate-200 dark:border-slate-800 flex items-center justify-between text-xs"
			>
				<div
					class="flex items-center gap-2 cursor-pointer select-none group"
					@click="toggleSelectAll"
				>
					<UiCheckbox
						:checked="
							isAllSelected
								? true
								: selectedItemIds.length > 0
									? 'indeterminate'
									: false
						"
						class="pointer-events-none"
					/>
					<span
						class="font-medium text-slate-700 dark:text-slate-300 group-hover:text-slate-900 dark:group-hover:text-slate-100 transition-colors"
					>
						Select All ({{ filteredItems.length }} available)
					</span>
				</div>
				<span class="text-slate-500 dark:text-slate-400">
					<span class="font-bold text-sky-600 dark:text-sky-400">{{
						selectedItemIds.length
					}}</span>
					items selected
				</span>
			</div>

			<!-- Items List Container -->
			<div
				class="flex-1 overflow-y-auto divide-y divide-slate-100 dark:divide-slate-800 min-h-[220px] max-h-[380px]"
			>
				<!-- Loading State -->
				<div v-if="store.loadingItems" class="p-8 text-center text-xs text-slate-400">
					Loading pipeline items...
				</div>

				<!-- Empty State -->
				<div
					v-else-if="filteredItems.length === 0"
					class="p-10 flex flex-col items-center justify-center text-center gap-2"
				>
					<span class="text-sm font-semibold text-slate-700 dark:text-slate-300">
						No items found
					</span>
					<span class="text-xs text-slate-400 max-w-sm">
						There are no items matching this criteria for the current pipeline hosts.
					</span>
				</div>

				<!-- Item Rows -->
				<div
					v-for="item in filteredItems"
					v-else
					:key="getItemId(item)"
					class="flex items-center justify-between px-6 py-2.5 hover:bg-slate-50 dark:hover:bg-slate-800/50 cursor-pointer transition-colors"
					@click="toggleItem(item)"
				>
					<div class="flex items-center gap-3 min-w-0">
						<UiCheckbox :checked="isItemChecked(item)" class="pointer-events-none" />
						<div class="flex flex-col min-w-0">
							<span
								class="text-xs font-semibold text-slate-800 dark:text-slate-200 truncate"
							>
								{{ item.name || item.resolved_name || item.item_id }}
							</span>
							<span class="text-[11px] text-slate-400 truncate">
								Host: {{ item.host_name || item.host_id || '-' }}
							</span>
						</div>
					</div>

					<div class="flex items-center gap-2 shrink-0">
						<!-- Item Type Badge -->
						<span
							v-if="item.type"
							class="px-2 py-0.5 rounded text-[10px] font-medium bg-slate-100 dark:bg-slate-800 text-slate-600 dark:text-slate-300"
						>
							{{ item.type }}
						</span>

						<!-- Status Dot -->
						<span
							class="w-2 h-2 rounded-full"
							:class="
								item.adm_status === 1
									? 'bg-emerald-500'
									: 'bg-slate-300 dark:bg-slate-600'
							"
							:title="item.adm_status === 1 ? 'Active' : 'Inactive'"
						/>
					</div>
				</div>
			</div>

			<!-- Footer -->
			<div
				class="px-6 py-3 border-t border-slate-200 dark:border-slate-800 flex items-center justify-between bg-slate-50/50 dark:bg-slate-950/40"
			>
				<UiButton variant="ghost" size="sm" @click="handleClose"> Cancel </UiButton>

				<UiButton size="sm" @click="handleApply">
					Apply ({{ selectedItemIds.length }} Items)
				</UiButton>
			</div>
		</div>
	</div>
</template>

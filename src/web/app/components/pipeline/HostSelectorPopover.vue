<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { usePipelineStore } from '@/stores/pipeline.store'
import type { IHostSelectorItem } from '@@/shared/types/pipeline.management.type'

const props = withDefaults(
	defineProps<{
		modelValue?: boolean
		inline?: boolean
	}>(),
	{
		modelValue: true,
		inline: false
	}
)

const emit = defineEmits<{
	(e: 'update:modelValue', val: boolean): void
	(e: 'close'): void
}>()

const pipelineStore = usePipelineStore()

const search = ref('')
const draftSelected = ref<string[]>([])

// Initialize draft from store
watch(
	() => props.modelValue,
	(isOpen) => {
		if (isOpen || props.inline) {
			draftSelected.value = [...pipelineStore.form.assignedHosts]
			search.value = ''
		}
	},
	{ immediate: true }
)

const filteredHosts = computed(() => {
	const q = search.value.toLowerCase().trim()
	return pipelineStore.availableHosts.filter((host: IHostSelectorItem) => {
		if (q === '') return true
		return host.name.toLowerCase().includes(q) || host.ip.toLowerCase().includes(q)
	})
})

function toggleHost(hostId: string) {
	const idx = draftSelected.value.indexOf(hostId)
	if (idx > -1) {
		draftSelected.value.splice(idx, 1)
	} else {
		draftSelected.value.push(hostId)
	}
}

function isHostChecked(hostId: string): boolean {
	return draftSelected.value.includes(hostId)
}

function selectAllFiltered() {
	const currentFilteredIds = filteredHosts.value.map((h) => h.id)
	const allInDraft = currentFilteredIds.every((id) => draftSelected.value.includes(id))

	if (allInDraft) {
		draftSelected.value = draftSelected.value.filter((id) => !currentFilteredIds.includes(id))
	} else {
		currentFilteredIds.forEach((id) => {
			if (!draftSelected.value.includes(id)) draftSelected.value.push(id)
		})
	}
}

function clearAll() {
	draftSelected.value = []
}

function applySelection() {
	pipelineStore.form.assignedHosts = [...draftSelected.value]
	close()
}

function close() {
	emit('update:modelValue', false)
	emit('close')
}
</script>

<template>
	<div
		v-if="inline || modelValue"
		:class="
			inline
				? 'w-full'
				: 'fixed inset-0 z-50 flex items-center justify-center p-4 bg-slate-900/40 backdrop-blur-xs'
		"
		@click.self="!inline && close()"
	>
		<div
			:class="[
				'w-full rounded-[10px] border border-slate-200 dark:border-slate-800 bg-white dark:bg-slate-900 flex flex-col overflow-hidden animate-in fade-in duration-150',
				inline ? 'shadow-xl' : 'max-w-[490px] shadow-2xl zoom-in-95'
			]"
		>
			<div
				class="flex items-center justify-between px-4 py-3 border-b border-slate-200 dark:border-slate-800"
			>
				<div class="flex items-center gap-2">
					<span class="text-xs font-semibold text-slate-800 dark:text-slate-100">
						Select Hosts
					</span>
					<span
						class="px-2 py-0.5 rounded text-[10px] font-medium bg-sky-100 dark:bg-sky-950 text-sky-600 dark:text-sky-400"
					>
						{{ draftSelected.length }} of
						{{ pipelineStore.availableHosts.length }} Selected
					</span>
				</div>
				<button
					type="button"
					class="w-6 h-6 flex items-center justify-center rounded text-slate-400 hover:text-slate-600 dark:hover:text-slate-200 text-sm"
					@click="close"
				>
					✕
				</button>
			</div>

			<div
				class="p-3 border-b border-slate-200 dark:border-slate-800 flex flex-col gap-2.5 bg-slate-50/50 dark:bg-slate-950/40"
			>
				<div class="flex items-center gap-2">
					<div class="relative flex-1">
						<UiIcon
							name="magnify"
							size="14px"
							class="absolute left-2.5 top-1/2 -translate-y-1/2 text-slate-400 pointer-events-none"
						/>
						<input
							v-model="search"
							type="text"
							placeholder="Search host by name or IP..."
							class="w-full pl-8 pr-3 py-1.5 text-xs bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-md focus:outline-none focus:border-sky-500 text-slate-800 dark:text-slate-100 placeholder:text-slate-400"
						/>
					</div>

					<button
						type="button"
						class="px-2.5 py-1.5 text-xs rounded border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 text-slate-700 dark:text-slate-300 hover:bg-slate-50 dark:hover:bg-slate-800 shrink-0 font-medium"
						@click="selectAllFiltered"
					>
						Select All
					</button>
				</div>
			</div>

			<div
				class="max-h-[220px] overflow-y-auto divide-y divide-slate-100 dark:divide-slate-800/80 p-1"
			>
				<div
					v-for="host in filteredHosts"
					:key="host.id"
					class="flex items-center justify-between px-3 py-2 rounded transition-colors cursor-pointer"
					:class="
						isHostChecked(host.id)
							? 'bg-sky-50/80 dark:bg-sky-950/40'
							: 'hover:bg-slate-50 dark:hover:bg-slate-800/50'
					"
					@click="toggleHost(host.id)"
				>
					<div class="flex items-center gap-2.5 min-w-0">
						<div
							class="w-4 h-4 rounded border flex items-center justify-center transition-colors"
							:class="
								isHostChecked(host.id)
									? 'bg-sky-600 border-sky-600 text-white'
									: 'border-slate-300 dark:border-slate-600 bg-white dark:bg-slate-900'
							"
						>
							<svg
								v-if="isHostChecked(host.id)"
								class="w-2.5 h-2.5"
								viewBox="0 0 12 12"
								fill="none"
							>
								<path
									d="M10 3L4.5 8.5L2 6"
									stroke="currentColor"
									stroke-width="2"
									stroke-linecap="round"
									stroke-linejoin="round"
								/>
							</svg>
						</div>
						<span
							class="text-xs truncate"
							:class="
								isHostChecked(host.id)
									? 'font-semibold text-slate-900 dark:text-slate-100'
									: 'font-normal text-slate-700 dark:text-slate-300'
							"
						>
							{{ host.name }}
						</span>
					</div>

					<span
						class="px-2 py-0.5 rounded text-[10px] font-mono shrink-0"
						:class="
							isHostChecked(host.id)
								? 'bg-sky-100 dark:bg-sky-950 text-sky-600 dark:text-sky-400 border border-sky-200 dark:border-sky-800'
								: 'bg-slate-100 dark:bg-slate-800 text-slate-500 dark:text-slate-400 border border-slate-200 dark:border-slate-700'
						"
					>
						{{ host.ip }}
					</span>
				</div>

				<div
					v-if="filteredHosts.length === 0"
					class="py-8 text-center text-xs text-slate-400 dark:text-slate-500"
				>
					No hosts found matching filter
				</div>
			</div>

			<div
				class="flex items-center justify-between px-4 py-3 border-t border-slate-200 dark:border-slate-800 bg-slate-50/50 dark:bg-slate-950/40"
			>
				<button
					type="button"
					class="px-3 py-1.5 text-xs rounded border border-slate-300 dark:border-slate-700 text-slate-700 dark:text-slate-300 bg-white dark:bg-slate-900 hover:bg-slate-50 dark:hover:bg-slate-800"
					@click="clearAll"
				>
					Clear All
				</button>
				<button
					type="button"
					class="px-3 py-1.5 text-xs rounded bg-sky-600 hover:bg-sky-700 text-white font-medium shadow-xs"
					@click="applySelection"
				>
					Apply Selection ({{ draftSelected.length }})
				</button>
			</div>
		</div>
	</div>
</template>

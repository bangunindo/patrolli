<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { ref } from 'vue'
import { onClickOutside } from '@vueuse/core'
import { Handle, Position } from '@vue-flow/core'
import type { IProcessorNodeData } from '@/stores/pipeline.builder.store'
import { usePipelineBuildStore } from '@/stores/pipeline.builder.store'

const props = defineProps<{
	id?: string
	data: IProcessorNodeData
}>()

const store = usePipelineBuildStore()
const showMenu = ref(false)
const menuRef = ref<HTMLElement | null>(null)

onClickOutside(menuRef, () => {
	showMenu.value = false
})

function toggleMenu(event: MouseEvent) {
	event.stopPropagation()
	showMenu.value = !showMenu.value
}

function handleDelete(event: MouseEvent) {
	event.stopPropagation()
	showMenu.value = false
	const targetId = props.id || props.data?.id || props.data?.node_id
	if (targetId) {
		store.removeNode(targetId)
	}
}

function handleAddProcessor() {
	const targetId = props.id || props.data?.id || props.data?.node_id
	if (!targetId) return

	const node = store.nodes.find((n: any) => n.id === targetId)
	if (node) {
		store.selectedNode = node
	}

	store.openCreateProcessorFormModal(targetId)
}

function handleAddSink() {
	store.addSinkNode()
}
</script>

<template>
	<!-- Node component matching Figma #2778:139776 / 2778:139777 -->
	<div class="flex flex-col items-center">
		<div
			class="relative w-[210px] rounded-[6px] border-[1.5px] border-[#FFA500] bg-white dark:bg-slate-900 shadow-[0px_1px_2px_0px_rgba(15,35,75,0.06)] flex flex-col p-3.5 gap-2.5 select-none transition-shadow hover:shadow-md"
		>
			<!-- 4 Connection Handles: Left (target), Right (primary source), Top, Bottom -->
			<Handle
				id="left"
				type="target"
				:position="Position.Left"
				class="!w-3.5 !h-3.5 !bg-white dark:!bg-slate-900 !border-[1.5px] !border-[#FFA500] !rounded-full z-10"
			/>
			<Handle
				id="right"
				type="source"
				:position="Position.Right"
				class="!w-3.5 !h-3.5 !bg-white dark:!bg-slate-900 !border-[1.5px] !border-[#FFA500] !rounded-full z-10"
			/>
			<Handle
				id="top"
				type="source"
				:position="Position.Top"
				class="!w-3.5 !h-3.5 !bg-white dark:!bg-slate-900 !border-[1.5px] !border-[#FFA500] !rounded-full z-10"
			/>
			<Handle
				id="bottom"
				type="source"
				:position="Position.Bottom"
				class="!w-3.5 !h-3.5 !bg-white dark:!bg-slate-900 !border-[1.5px] !border-[#FFA500] !rounded-full z-10"
			/>

			<!-- Row 1: Header (Icon, Title, Badge, Options) -->
			<div class="flex items-center justify-between gap-1 w-full">
				<div class="flex items-center gap-1.5 flex-1 min-w-0">
					<!-- Tune / Processor Icon -->
					<svg
						width="18"
						height="18"
						viewBox="0 0 24 24"
						fill="none"
						stroke="#FFA500"
						stroke-width="2"
						stroke-linecap="round"
						stroke-linejoin="round"
						class="shrink-0"
					>
						<line x1="4" y1="21" x2="4" y2="14" />
						<line x1="4" y1="10" x2="4" y2="3" />
						<line x1="12" y1="21" x2="12" y2="12" />
						<line x1="12" y1="8" x2="12" y2="3" />
						<line x1="20" y1="21" x2="20" y2="16" />
						<line x1="20" y1="12" x2="20" y2="3" />
						<line x1="1" y1="14" x2="7" y2="14" />
						<line x1="9" y1="8" x2="15" y2="8" />
						<line x1="17" y1="16" x2="23" y2="16" />
					</svg>

					<!-- Title -->
					<span class="text-[11px] font-medium text-black dark:text-white truncate">
						{{ data.name || 'Processor' }}
					</span>
				</div>

				<!-- Processor Badge -->
				<div class="px-1.5 py-0.5 rounded-[6px] bg-[#FFEDCC] dark:bg-amber-950/60 shrink-0">
					<span
						class="text-[9px] font-bold text-[#FFA500] dark:text-amber-300 leading-tight block"
					>
						Processor
					</span>
				</div>

				<!-- 3-Dots Menu -->
				<div ref="menuRef" class="relative shrink-0">
					<button
						type="button"
						class="text-slate-400 hover:text-slate-600 dark:hover:text-slate-200 transition-colors shrink-0 p-1 rounded hover:bg-slate-100 dark:hover:bg-slate-800"
						title="Node options"
						@click="toggleMenu"
					>
						<svg width="15" height="15" viewBox="0 0 24 24" fill="currentColor">
							<circle cx="12" cy="5" r="2" />
							<circle cx="12" cy="12" r="2" />
							<circle cx="12" cy="19" r="2" />
						</svg>
					</button>

					<!-- Dropdown Menu -->
					<div
						v-if="showMenu"
						class="absolute right-0 top-full mt-1 w-28 bg-white dark:bg-slate-800 rounded-md shadow-lg border border-slate-200 dark:border-slate-700 py-1 z-50 animate-in fade-in zoom-in-95 duration-100"
					>
						<button
							type="button"
							class="w-full flex items-center gap-2 px-3 py-1.5 text-xs text-rose-600 dark:text-rose-400 hover:bg-rose-50 dark:hover:bg-rose-950/50 transition-colors text-left"
							@click="handleDelete"
						>
							<svg
								width="13"
								height="13"
								viewBox="0 0 24 24"
								fill="none"
								stroke="currentColor"
								stroke-width="2"
								stroke-linecap="round"
								stroke-linejoin="round"
							>
								<path
									d="M3 6h18M19 6v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6m3 0V4a2 2 0 012-2h4a2 2 0 012 2v2M10 11v6M14 11v6"
								/>
							</svg>
							<span>Delete</span>
						</button>
					</div>
				</div>
			</div>

			<!-- Divider -->
			<div class="w-full h-px bg-[#D8D9DB] dark:bg-slate-700" />

			<!-- Row 2: Subtitle / Summary (X Items) -->
			<div class="text-[10px] font-medium text-[#475569] dark:text-slate-400 leading-none">
				{{ data.itemCount ?? 0 }} Items
			</div>

			<!-- Row 3: Active Status Indicator -->
			<div class="flex items-center gap-1.5">
				<span class="w-[7px] h-[7px] rounded-full bg-[#32B062] shrink-0" />
				<span
					class="text-[10px] font-medium text-[#475569] dark:text-slate-300 leading-none"
				>
					{{
						data.activeCount ??
						(data.processors
							? data.processors.filter((p: any) => p.status !== false).length
							: (data.processCount ?? 0))
					}}
					Active Processor
				</span>
			</div>
		</div>

		<!-- Bottom Action Buttons matching Figma #2778:139807 & 2778:139808 (+ Processor & + Sink) -->
		<div class="flex flex-col items-center gap-1.5 mt-3 select-none">
			<button
				type="button"
				class="inline-flex items-center gap-1 text-[11px] font-medium text-[#0284C7] hover:text-sky-700 dark:text-sky-400 dark:hover:text-sky-300 transition-colors cursor-pointer"
				@click.stop="handleAddProcessor"
			>
				<svg
					width="12"
					height="12"
					viewBox="0 0 24 24"
					fill="none"
					stroke="currentColor"
					stroke-width="2.5"
					stroke-linecap="round"
					stroke-linejoin="round"
				>
					<path d="M12 5v14M5 12h14" />
				</svg>
				<span>Processor</span>
			</button>
			<button
				type="button"
				class="inline-flex items-center gap-1 text-[11px] font-medium text-[#0284C7] hover:text-sky-700 dark:text-sky-400 dark:hover:text-sky-300 transition-colors cursor-pointer"
				@click.stop="handleAddSink"
			>
				<svg
					width="12"
					height="12"
					viewBox="0 0 24 24"
					fill="none"
					stroke="currentColor"
					stroke-width="2.5"
					stroke-linecap="round"
					stroke-linejoin="round"
				>
					<path d="M12 5v14M5 12h14" />
				</svg>
				<span>Sink</span>
			</button>
		</div>
	</div>
</template>

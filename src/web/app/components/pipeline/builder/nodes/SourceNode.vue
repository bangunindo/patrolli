<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { ref } from 'vue'
import { onClickOutside } from '@vueuse/core'
import { Handle, Position } from '@vue-flow/core'
import type { ISourceNodeData } from '@/stores/pipeline.builder.store'
import { usePipelineBuildStore } from '@/stores/pipeline.builder.store'

const props = defineProps<{
	id?: string
	data: ISourceNodeData
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
</script>

<template>
	<!-- Node component matching Figma #1998:39970 (Property 1=host) in node 2778:139774 -->
	<div
		class="relative w-[256px] rounded-[8px] border-2 border-[#0284C7] bg-white dark:bg-slate-900 shadow-[0px_1px_2px_0px_rgba(15,35,75,0.06)] flex flex-col p-4 gap-3 select-none transition-shadow hover:shadow-md"
	>
		<!-- 4 Connection Handles: Right (default source handle), Top, Bottom, Left (target) -->
		<Handle
			id="right"
			type="source"
			:position="Position.Right"
			class="!w-4 !h-4 !bg-white dark:!bg-slate-900 !border-2 !border-[#0284C7] !rounded-full z-10"
		/>
		<Handle
			id="top"
			type="source"
			:position="Position.Top"
			class="!w-4 !h-4 !bg-white dark:!bg-slate-900 !border-2 !border-[#0284C7] !rounded-full z-10"
		/>
		<Handle
			id="bottom"
			type="source"
			:position="Position.Bottom"
			class="!w-4 !h-4 !bg-white dark:!bg-slate-900 !border-2 !border-[#0284C7] !rounded-full z-10"
		/>
		<Handle
			id="left"
			type="target"
			:position="Position.Left"
			class="!w-4 !h-4 !bg-white dark:!bg-slate-900 !border-2 !border-[#0284C7] !rounded-full z-10"
		/>

		<!-- Row 1: Header (Icon, Title, Telemetry Badge, Options) -->
		<div class="flex items-center justify-between gap-1 w-full">
			<div class="flex items-center gap-1.5 flex-1 min-w-0">
				<!-- Server Icon -->
				<svg
					width="22"
					height="22"
					viewBox="0 0 24 24"
					fill="none"
					stroke="#0284C7"
					stroke-width="1.8"
					stroke-linecap="round"
					stroke-linejoin="round"
					class="shrink-0"
				>
					<rect x="2" y="2" width="20" height="8" rx="2" ry="2" />
					<rect x="2" y="14" width="20" height="8" rx="2" ry="2" />
					<line x1="6" y1="6" x2="6.01" y2="6" />
					<line x1="6" y1="18" x2="6.01" y2="18" />
				</svg>

				<!-- Node Title -->
				<span class="text-xs font-medium text-black dark:text-white truncate">
					{{ data.title || 'Source' }}
				</span>
			</div>

			<!-- Telemetry Badge (Logs) -->
			<div
				class="flex items-center gap-1 px-1.5 py-0.5 rounded-[8px] bg-[#E0F2FE] dark:bg-sky-950/60 shrink-0"
			>
				<svg
					width="12"
					height="12"
					viewBox="0 0 24 24"
					fill="none"
					stroke="#0284C7"
					stroke-width="2.5"
					stroke-linecap="round"
					stroke-linejoin="round"
				>
					<line x1="8" y1="6" x2="21" y2="6" />
					<line x1="8" y1="12" x2="21" y2="12" />
					<line x1="8" y1="18" x2="21" y2="18" />
					<circle cx="4" cy="6" r="1.5" fill="#0284C7" />
					<circle cx="4" cy="12" r="1.5" fill="#0284C7" />
					<circle cx="4" cy="18" r="1.5" fill="#0284C7" />
				</svg>
				<span class="text-xs font-normal text-[#0284C7] dark:text-sky-300 leading-none"
					>Logs</span
				>
			</div>

			<!-- 3-Dots Menu -->
			<div ref="menuRef" class="relative shrink-0">
				<button
					type="button"
					class="text-slate-400 hover:text-slate-600 dark:hover:text-slate-200 transition-colors shrink-0 p-1 rounded hover:bg-slate-100 dark:hover:bg-slate-800"
					title="Node options"
					@click="toggleMenu"
				>
					<svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor">
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

		<!-- Divider 1 -->
		<div class="w-full h-px bg-[#D8D9DB] dark:bg-slate-700" />

		<!-- Row 2: Subtitle / Summary (X Host · Y Item) -->
		<div class="text-[10px] font-medium text-[#475569] dark:text-slate-400 leading-none">
			{{ data.hostCount ?? 0 }} Host &nbsp;·&nbsp; {{ data.itemCount ?? 0 }} Item
		</div>

		<!-- Divider 2 -->
		<div class="w-full h-px bg-[#D8D9DB] dark:bg-slate-700" />

		<!-- Row 3: Health Indicators (Healthy, Delayed, Failed) -->
		<div class="flex items-center gap-3">
			<div class="flex items-center gap-1">
				<span class="w-[7px] h-[7px] rounded-full bg-[#32B062] shrink-0" />
				<span
					class="text-[10px] font-medium text-[#475569] dark:text-slate-300 leading-none"
				>
					{{ data.health?.healthy ?? 0 }} Healthy
				</span>
			</div>
			<div class="flex items-center gap-1">
				<span class="w-[7px] h-[7px] rounded-full bg-[#E69B00] shrink-0" />
				<span
					class="text-[10px] font-medium text-[#475569] dark:text-slate-300 leading-none"
				>
					{{ data.health?.warning ?? 0 }} Delayed
				</span>
			</div>
			<div class="flex items-center gap-1">
				<span class="w-[7px] h-[7px] rounded-full bg-[#B91A24] shrink-0" />
				<span
					class="text-[10px] font-medium text-[#475569] dark:text-slate-300 leading-none"
				>
					{{ data.health?.error ?? 0 }} Failed
				</span>
			</div>
		</div>
	</div>
</template>

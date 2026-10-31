<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed } from 'vue'
import { useVueFlow } from '@vue-flow/core'
import { usePipelineBuildStore } from '@/stores/pipeline.builder.store'

const { zoomIn, zoomOut, fitView, viewport } = useVueFlow('pipeline-canvas')
const store = usePipelineBuildStore()

const zoomPercent = computed(() => {
	const z = viewport.value?.zoom ?? 1
	return `${Math.round(z * 100)}%`
})

function handleZoomIn() {
	zoomIn({ duration: 250 })
}

function handleZoomOut() {
	zoomOut({ duration: 250 })
}

function handleFitView() {
	fitView({ padding: 0.25, duration: 350 })
}

function handleAutoLayout() {
	store.autoLayout()
	setTimeout(() => {
		fitView({ padding: 0.25, duration: 350 })
	}, 60)
}
</script>

<template>
	<div class="absolute left-6 bottom-6 z-10 flex items-center gap-1.5 select-none">
		<!-- Zoom controls group: [- | 100% | +] -->
		<div
			class="flex items-center h-[33px] bg-white dark:bg-slate-900 border border-[#D8D9DB] dark:border-slate-700 rounded-[3px] shadow-xs overflow-hidden"
		>
			<button
				type="button"
				class="w-[33px] h-full flex items-center justify-center text-[#1D2A44] dark:text-slate-200 text-sm font-medium hover:bg-slate-50 dark:hover:bg-slate-800 transition-colors"
				title="Zoom Out"
				@click="handleZoomOut"
			>
				&minus;
			</button>

			<div
				class="min-w-[43.5px] px-2 h-full flex items-center justify-center border-x border-[#D9E2EE] dark:border-slate-700 text-[#1D2A44] dark:text-slate-200 text-[10.5px] font-semibold leading-none"
			>
				{{ zoomPercent }}
			</div>

			<button
				type="button"
				class="w-[33px] h-full flex items-center justify-center text-[#1D2A44] dark:text-slate-200 text-sm font-medium hover:bg-slate-50 dark:hover:bg-slate-800 transition-colors"
				title="Zoom In"
				@click="handleZoomIn"
			>
				+
			</button>
		</div>

		<!-- Fit to Screen Button -->
		<button
			type="button"
			class="w-[33px] h-[33px] flex items-center justify-center bg-white dark:bg-slate-900 border border-[#D8D9DB] dark:border-slate-700 rounded-[3px] hover:bg-slate-50 dark:hover:bg-slate-800 transition-colors shadow-xs"
			title="Fit to Screen"
			@click="handleFitView"
		>
			<svg
				width="14"
				height="14"
				viewBox="0 0 14 14"
				fill="none"
				class="stroke-[#10182B] dark:stroke-slate-200"
			>
				<path
					d="M8.75 1.75H12.25V5.25"
					stroke-width="1.17"
					stroke-linecap="round"
					stroke-linejoin="round"
				/>
				<path
					d="M5.25 12.25H1.75V8.75"
					stroke-width="1.17"
					stroke-linecap="round"
					stroke-linejoin="round"
				/>
				<path
					d="M12.25 1.75L8.16667 5.83333"
					stroke-width="1.17"
					stroke-linecap="round"
					stroke-linejoin="round"
				/>
				<path
					d="M1.75 12.25L5.83333 8.16667"
					stroke-width="1.17"
					stroke-linecap="round"
					stroke-linejoin="round"
				/>
			</svg>
		</button>

		<!-- Auto Layout Button -->
		<button
			type="button"
			class="h-[33px] px-3 flex items-center gap-2 bg-white dark:bg-slate-900 border border-[#D8D9DB] dark:border-slate-700 rounded-[3px] hover:bg-slate-50 dark:hover:bg-slate-800 transition-colors shadow-xs"
			title="Auto Layout"
			@click="handleAutoLayout"
		>
			<svg
				width="12"
				height="12"
				viewBox="0 0 12 12"
				fill="none"
				class="fill-black dark:fill-white"
			>
				<path
					d="M8.5 2H10C10.55 2 11 2.45 11 3V4H10V3H8.5V2ZM2 4V3H3.5V2H2C1.45 2 1 2.45 1 3V4H2ZM10 8V9H8.5V10H10C10.55 10 11 9.55 11 9V8H10ZM3.5 9H2V8H1V9C1 9.55 1.45 10 2 10H3.5V9ZM9 4H3V8H9V4Z"
				/>
			</svg>
			<span class="text-[10px] font-bold text-slate-900 dark:text-slate-100 leading-none"
				>Auto Layout</span
			>
		</button>
	</div>
</template>

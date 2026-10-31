<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { usePipelineBuildStore } from '@/stores/pipeline.builder.store'

const store = usePipelineBuildStore()

function onDragStart(event: DragEvent, nodeType: string) {
	if (event.dataTransfer) {
		event.dataTransfer.setData('application/vueflow', nodeType)
		event.dataTransfer.effectAllowed = 'move'
	}
}

function handleAddSource() {
	store.addSourceNode()
}

function handleAddProcessor() {
	store.addProcessorNode(undefined, undefined, 'Processor Group')
}

function handleAddSink() {
	store.addSinkNode()
}
</script>

<template>
	<div
		class="absolute left-1/2 -translate-x-1/2 bottom-6 z-20 flex items-center gap-1.5 p-1.5 bg-white/95 dark:bg-slate-900/95 backdrop-blur-md border border-slate-200/90 dark:border-slate-800/90 rounded-full shadow-[0_8px_30px_rgba(15,23,42,0.12)] select-none"
	>
		<!-- Source Button -->
		<button
			type="button"
			draggable="true"
			class="group flex items-center gap-2 px-3 py-1.5 rounded-full text-xs font-medium text-slate-700 dark:text-slate-200 hover:text-sky-600 dark:hover:text-sky-400 hover:bg-sky-50 dark:hover:bg-sky-950/60 border border-transparent hover:border-sky-200 dark:hover:border-sky-900 transition-all cursor-grab active:cursor-grabbing"
			title="Click to add or drag onto canvas"
			@dragstart="onDragStart($event, 'source')"
			@click="handleAddSource"
		>
			<span
				class="w-5 h-5 rounded-full bg-sky-100 dark:bg-sky-950/80 text-sky-600 dark:text-sky-400 flex items-center justify-center shrink-0 group-hover:scale-110 transition-transform"
			>
				<svg
					width="12"
					height="12"
					viewBox="0 0 24 24"
					fill="none"
					stroke="currentColor"
					stroke-width="2.2"
					stroke-linecap="round"
					stroke-linejoin="round"
				>
					<rect x="2" y="2" width="20" height="8" rx="2" ry="2" />
					<rect x="2" y="14" width="20" height="8" rx="2" ry="2" />
					<line x1="6" y1="6" x2="6.01" y2="6" />
					<line x1="6" y1="18" x2="6.01" y2="18" />
				</svg>
			</span>
			<span class="font-semibold text-slate-800 dark:text-slate-200">Source</span>
		</button>

		<div class="w-[1px] h-5 bg-slate-200 dark:bg-slate-800" />

		<!-- Processor Group Button -->
		<button
			type="button"
			draggable="true"
			class="group flex items-center gap-2 px-3 py-1.5 rounded-full text-xs font-medium text-slate-700 dark:text-slate-200 hover:text-amber-600 dark:hover:text-amber-400 hover:bg-amber-50 dark:hover:bg-amber-950/60 border border-transparent hover:border-amber-200 dark:hover:border-amber-900 transition-all cursor-grab active:cursor-grabbing"
			title="Click to add or drag onto canvas"
			@dragstart="onDragStart($event, 'processor')"
			@click="handleAddProcessor"
		>
			<span
				class="w-5 h-5 rounded-full bg-amber-100 dark:bg-amber-950/80 text-amber-600 dark:text-amber-400 flex items-center justify-center shrink-0 group-hover:scale-110 transition-transform"
			>
				<svg
					width="12"
					height="12"
					viewBox="0 0 24 24"
					fill="none"
					stroke="currentColor"
					stroke-width="2.2"
					stroke-linecap="round"
					stroke-linejoin="round"
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
			</span>
			<span class="font-semibold text-slate-800 dark:text-slate-200">Processor Group</span>
		</button>

		<div class="w-[1px] h-5 bg-slate-200 dark:bg-slate-800" />

		<!-- Sink Button -->
		<button
			type="button"
			draggable="true"
			class="group flex items-center gap-2 px-3 py-1.5 rounded-full text-xs font-medium text-slate-700 dark:text-slate-200 hover:text-rose-600 dark:hover:text-rose-400 hover:bg-rose-50 dark:hover:bg-rose-950/60 border border-transparent hover:border-rose-200 dark:hover:border-rose-900 transition-all cursor-grab active:cursor-grabbing"
			title="Click to add or drag onto canvas"
			@dragstart="onDragStart($event, 'sink')"
			@click="handleAddSink"
		>
			<span
				class="w-5 h-5 rounded-full bg-rose-100 dark:bg-rose-950/80 text-rose-600 dark:text-rose-400 flex items-center justify-center shrink-0 group-hover:scale-110 transition-transform"
			>
				<svg
					width="12"
					height="12"
					viewBox="0 0 24 24"
					fill="none"
					stroke="currentColor"
					stroke-width="2.2"
					stroke-linecap="round"
					stroke-linejoin="round"
				>
					<ellipse cx="12" cy="5" rx="9" ry="3" />
					<path d="M21 12c0 1.66-4 3-9 3s-9-1.34-9-3" />
					<path d="M3 5v14c0 1.66 4 3 9 3s9-1.34 9-3V5" />
				</svg>
			</span>
			<span class="font-semibold text-slate-800 dark:text-slate-200">Sink</span>
		</button>
	</div>
</template>

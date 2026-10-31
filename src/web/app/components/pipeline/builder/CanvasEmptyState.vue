<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { usePipelineBuildStore } from '@/stores/pipeline.builder.store'

const store = usePipelineBuildStore()

function handleCreate() {
	store.addSourceNode()
}
</script>

<template>
	<div
		class="canvas-empty-state flex flex-col items-center justify-center p-8 text-center select-none"
	>
		<!-- Title -->
		<h3 class="text-base font-bold text-slate-900 dark:text-slate-100 mb-6">No Pipeline yet</h3>

		<!-- Flow Nodes Illustration (Figma #509:59767) -->
		<div class="flex items-center justify-center gap-3 mb-6">
			<!-- Node 1: Blue (Source) -->
			<div
				class="w-12 h-12 rounded-lg bg-sky-500/90 shadow-xs flex items-center justify-center"
			></div>

			<!-- Arrow 1 -->
			<div class="text-slate-400 dark:text-slate-500">
				<svg
					width="18"
					height="18"
					viewBox="0 0 24 24"
					fill="none"
					stroke="currentColor"
					stroke-width="2.2"
					stroke-linecap="round"
					stroke-linejoin="round"
				>
					<path d="M5 12h14" />
					<path d="m12 5 7 7-7 7" />
				</svg>
			</div>

			<!-- Node 2: Teal -->
			<div
				class="w-12 h-12 rounded-lg bg-teal-500/90 shadow-xs flex items-center justify-center"
			></div>

			<!-- Arrow 2 -->
			<div class="text-slate-400 dark:text-slate-500">
				<svg
					width="18"
					height="18"
					viewBox="0 0 24 24"
					fill="none"
					stroke="currentColor"
					stroke-width="2.2"
					stroke-linecap="round"
					stroke-linejoin="round"
				>
					<path d="M5 12h14" />
					<path d="m12 5 7 7-7 7" />
				</svg>
			</div>

			<!-- Node 3: Purple -->
			<div
				class="w-12 h-12 rounded-lg bg-purple-500/90 shadow-xs flex items-center justify-center"
			></div>

			<!-- Arrow 3 -->
			<div class="text-slate-400 dark:text-slate-500">
				<svg
					width="18"
					height="18"
					viewBox="0 0 24 24"
					fill="none"
					stroke="currentColor"
					stroke-width="2.2"
					stroke-linecap="round"
					stroke-linejoin="round"
				>
					<path d="M5 12h14" />
					<path d="m12 5 7 7-7 7" />
				</svg>
			</div>

			<!-- Node 4: Orange (Sink) -->
			<div
				class="w-12 h-12 rounded-lg bg-amber-500/90 shadow-xs flex items-center justify-center"
			></div>
		</div>

		<!-- Description Guide -->
		<p class="text-sm text-slate-500 dark:text-slate-400 max-w-[280px] leading-relaxed mb-6">
			Build your pipeline by adding a source<br />
			and connect the nodes.
		</p>

		<!-- Action Button matching Figma #509:59779 -->
		<UiButton class="px-6 gap-2" @click="handleCreate">
			<UiIcon name="plus" size="16px" />
			<span>Create</span>
		</UiButton>
	</div>
</template>

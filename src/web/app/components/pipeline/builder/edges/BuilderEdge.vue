<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { BaseEdge, EdgeLabelRenderer, getBezierPath, type EdgeProps } from '@vue-flow/core'
import { computed } from 'vue'
import { usePipelineBuildStore } from '@/stores/pipeline.builder.store'

const props = defineProps<EdgeProps>()
const store = usePipelineBuildStore()

const pathData = computed(() => {
	return getBezierPath({
		sourceX: props.sourceX,
		sourceY: props.sourceY,
		sourcePosition: props.sourcePosition,
		targetX: props.targetX,
		targetY: props.targetY,
		targetPosition: props.targetPosition
	})
})

const isSourceToProcessor = computed(() => {
	const fromNode = store.nodes.find((n: any) => n.id === props.source)
	const toNode = store.nodes.find((n: any) => n.id === props.target)
	return fromNode?.type === 'source' && toNode?.type === 'processor'
})

function handleAddNodeBetween() {
	const [_edgePath, edgeCenterX, edgeCenterY] = pathData.value
	store.insertProcessorBetween(props.id, edgeCenterX - 105, edgeCenterY - 40)
}

function handleLabelClick() {
	if (isSourceToProcessor.value) {
		store.openEdgeItemSelector(props.id)
	}
}
</script>

<template>
	<!-- Invisible wider stroke path for easy clicking of the line -->
	<path
		v-if="isSourceToProcessor"
		:d="pathData[0]"
		fill="none"
		stroke="transparent"
		stroke-width="24"
		class="cursor-pointer pointer-events-auto"
		title="Click to select items for this connection"
		@click.stop="handleLabelClick"
	/>

	<!-- Base Bezier Edge Line matching Figma 2778:139774 -->
	<BaseEdge
		:id="id"
		:style="style"
		:path="pathData[0]"
		:marker-end="markerEnd"
		:class="{ 'cursor-pointer': isSourceToProcessor }"
		@click.stop="handleLabelClick"
	/>

	<!-- Edge Label / Interactive Ornaments (Label Badge or Plus Button) -->
	<EdgeLabelRenderer>
		<div
			:style="{
				transform: `translate(-50%, -50%) translate(${pathData[1]}px,${pathData[2]}px)`
			}"
			class="nodrag nopan pointer-events-auto absolute z-[2] flex items-center justify-center"
		>
			<!-- Label Badge (e.g. '4 Items' matching Figma #2778:139809) -->
			<div
				v-if="data?.label"
				class="px-2 py-0.5 rounded-full bg-white dark:bg-slate-800 border border-[#BAE6FD] dark:border-sky-800 hover:border-sky-500 text-[10px] font-medium text-[#475569] dark:text-slate-300 hover:text-sky-600 dark:hover:text-sky-400 shadow-xs flex items-center justify-center select-none cursor-pointer transition-all hover:scale-105 active:scale-95"
				title="Click to edit selected items"
				@click.stop="handleLabelClick"
			>
				{{ data.label }}
			</div>

			<!-- Plus Button (matching Figma #2778:139810) -->
			<button
				v-else-if="data?.showAddButton"
				type="button"
				title="Add intermediate node"
				class="w-4 h-4 rounded-[3px] bg-primary hover:bg-primary/90 text-primary-foreground flex items-center justify-center shadow-xs cursor-pointer transition-transform hover:scale-115 active:scale-95"
				@click.stop="handleAddNodeBetween"
			>
				<svg
					width="10"
					height="10"
					viewBox="0 0 24 24"
					fill="none"
					stroke="currentColor"
					stroke-width="3"
					stroke-linecap="round"
					stroke-linejoin="round"
				>
					<line x1="12" y1="5" x2="12" y2="19" />
					<line x1="5" y1="12" x2="19" y2="12" />
				</svg>
			</button>
		</div>
	</EdgeLabelRenderer>
</template>

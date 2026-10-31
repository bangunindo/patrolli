<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { onMounted, onUnmounted } from 'vue'
import { useVueFlow, VueFlow } from '@vue-flow/core'
import { Background } from '@vue-flow/background'
import '@vue-flow/core/dist/style.css'
import '@vue-flow/core/dist/theme-default.css'

import SourceNode from '@/components/pipeline/builder/nodes/SourceNode.vue'
import BuilderProcessorNode from '@/components/pipeline/builder/nodes/BuilderProcessorNode.vue'
import BuilderSinkNode from '@/components/pipeline/builder/nodes/BuilderSinkNode.vue'
import BuilderEdge from '@/components/pipeline/builder/edges/BuilderEdge.vue'
import CanvasBottomControls from '@/components/pipeline/builder/CanvasBottomControls.vue'
import CanvasNodeToolbar from '@/components/pipeline/builder/CanvasNodeToolbar.vue'
import CanvasEmptyState from '@/components/pipeline/builder/CanvasEmptyState.vue'
import EdgeItemsModal from '@/components/pipeline/builder/modals/EdgeItemsModal.vue'
import { usePipelineBuildStore } from '@/stores/pipeline.builder.store'

const store = usePipelineBuildStore()
const {
	project,
	onConnect,
	onNodeClick,
	onPaneClick,
	onEdgeClick,
	onNodeDragStop,
	getSelectedNodes
} = useVueFlow('pipeline-canvas')

onConnect((params) => {
	store.connectNodes(
		params.source,
		params.target,
		params.sourceHandle || 'right',
		params.targetHandle || 'left'
	)
})

onNodeDragStop(() => {
	store.markDirty()
})

onNodeClick(({ node }) => {
	store.selectedNode = node
})

onEdgeClick(({ edge }) => {
	const fromNode = store.nodes.find((n: any) => n.id === edge.source)
	const toNode = store.nodes.find((n: any) => n.id === edge.target)
	if (fromNode?.type === 'source' && toNode?.type === 'processor') {
		store.openEdgeItemSelector(edge.id)
	}
})

onPaneClick(() => {
	store.selectedNode = undefined
})

function onDragOver(event: DragEvent) {
	event.preventDefault()
	if (event.dataTransfer) {
		event.dataTransfer.dropEffect = 'move'
	}
}

function onDrop(event: DragEvent) {
	event.preventDefault()
	const nodeType = event.dataTransfer?.getData('application/vueflow')
	if (!nodeType) return

	const canvasBounds = (event.currentTarget as HTMLElement).getBoundingClientRect()
	const position = project({
		x: event.clientX - canvasBounds.left,
		y: event.clientY - canvasBounds.top
	})

	if (nodeType === 'source') {
		store.addSourceNode(position.x, position.y)
	} else if (nodeType === 'processor') {
		store.addProcessorNode(position.x, position.y, 'Processor Group')
	} else if (nodeType === 'sink') {
		store.addSinkNode(position.x, position.y)
	}
}

function handleKeyDown(e: KeyboardEvent) {
	if (
		(e.target as HTMLElement)?.tagName === 'INPUT' ||
		(e.target as HTMLElement)?.tagName === 'TEXTAREA' ||
		(e.target as HTMLElement)?.isContentEditable
	) {
		return
	}

	if (e.key === 'Delete' || e.key === 'Backspace') {
		const nodesToDelete = new Set<string>()

		const vueFlowSelected = getSelectedNodes.value || []
		for (const n of vueFlowSelected) {
			if (n?.id) nodesToDelete.add(n.id)
		}

		if (store.selectedNode?.id) {
			nodesToDelete.add(store.selectedNode.id)
		}

		if (nodesToDelete.size > 0) {
			e.preventDefault()
			for (const id of nodesToDelete) {
				store.removeNode(id)
			}
		}
	}
}

onMounted(() => {
	window.addEventListener('keydown', handleKeyDown)
})

onUnmounted(() => {
	window.removeEventListener('keydown', handleKeyDown)
	store.selectedNode = undefined
})
</script>

<template>
	<div
		class="w-full h-full relative overflow-hidden bg-slate-50 dark:bg-slate-950"
		@dragover="onDragOver"
		@drop="onDrop"
	>
		<VueFlow
			v-model:nodes="store.nodes"
			v-model:edges="store.edges"
			fit-view-on-init
			class="h-full w-full"
			:default-viewport="{ zoom: 1, x: 0, y: 0 }"
			:min-zoom="0.2"
			:max-zoom="4"
			@node-drag-stop="store.markDirty"
		>
			<Background pattern-color="#94A3B8" :gap="16" :size="1" />
			<CanvasBottomControls />
			<CanvasNodeToolbar />

			<!-- Custom Node Renderers matching Figma 2778:139774 -->
			<template #node-source="props">
				<SourceNode :id="props.id" :data="props.data" />
			</template>

			<template #node-processor="props">
				<BuilderProcessorNode :id="props.id" :data="props.data" />
			</template>

			<template #node-sink="props">
				<BuilderSinkNode :id="props.id" :data="props.data" />
			</template>

			<!-- Custom Edge Renderer matching Figma 2778:139774 -->
			<template #edge-custom="props">
				<BuilderEdge v-bind="props" />
			</template>
		</VueFlow>

		<!-- Empty State Overlay when no nodes in canvas -->
		<div
			v-if="store.isCanvasEmpty && !store.loading.canvas"
			class="absolute inset-0 z-10 flex items-center justify-center pointer-events-none"
		>
			<div class="pointer-events-auto">
				<CanvasEmptyState />
			</div>
		</div>

		<EdgeItemsModal />
	</div>
</template>

<style>
/* Vue Flow custom connector line & handle overrides matching Figma #2778:139774 */
.vue-flow__edge-path {
	stroke: #0284c7;
	stroke-width: 2.5;
	transition:
		stroke-width 0.15s ease,
		stroke 0.15s ease;
}
.vue-flow__edge:hover .vue-flow__edge-path {
	stroke: #0284c7;
	stroke-width: 3;
}
.vue-flow__edge.selected .vue-flow__edge-path {
	stroke: #0284c7;
	stroke-width: 3;
}
.vue-flow__edge-marker path {
	fill: #0284c7 !important;
	stroke: #0284c7 !important;
}

/* Handle connection styling matching Figma #2778:139774 (No hover animation, centered on border) */
.vue-flow__handle {
	background: #ffffff !important;
	border-radius: 9999px !important;
	cursor: crosshair;
}

.vue-flow__handle-top {
	top: 0 !important;
	left: 50% !important;
	transform: translate(-50%, -50%) !important;
}

.vue-flow__handle-bottom {
	bottom: 0 !important;
	left: 50% !important;
	transform: translate(-50%, 50%) !important;
}

.vue-flow__handle-left {
	top: 50% !important;
	left: 0 !important;
	transform: translate(-50%, -50%) !important;
}

.vue-flow__handle-right {
	top: 50% !important;
	right: 0 !important;
	transform: translate(50%, -50%) !important;
}

.dark .vue-flow__handle {
	background: #0f172a !important;
}
.dark .vue-flow__edge-path {
	stroke: #38bdf8;
}
.dark .vue-flow__edge-marker path {
	fill: #38bdf8 !important;
	stroke: #38bdf8 !important;
}

/* Node Stacking: Node cards must always sit above connector lines and intermediate buttons */
.vue-flow__node {
	z-index: 10;
}
.vue-flow__node.selected {
	z-index: 15;
}
</style>

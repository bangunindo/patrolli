<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { onMounted, onUnmounted } from 'vue'
import { useRoute } from 'vue-router'
import appHelper from '@/app/helpers/app.helper'
import CanvasTopNav from '@/components/pipeline/builder/CanvasTopNav.vue'
import BuilderCanvas from '@/components/pipeline/builder/BuilderCanvas.vue'
import NodeConfigDrawer from '@/components/pipeline/builder/NodeConfigDrawer.vue'
import ProcessorGroupDrawer from '@/components/pipeline/builder/ProcessorGroupDrawer.vue'
import PreviewDataModal from '@/components/pipeline/builder/modals/PreviewDataModal.vue'
import { useAppStore } from '@/stores/app.store'
import { useAuthStore } from '@/stores/auth.store'
import { usePipelineBuildStore } from '@/stores/pipeline.builder.store'

definePageMeta({
	layout: 'default',
	requiresAuth: true
})

const route = useRoute()
const appStore = useAppStore()
const authStore = useAuthStore()
const store = usePipelineBuildStore()

onMounted(async () => {
	store.selectedNode = undefined
	const pId = String(route.params.pipeline_id)
	const cId = String(route.params.canvas_id)
	const clusterUrl = route.query.cluster_url ? String(route.query.cluster_url) : undefined
	if (!authStore.isClusterReadonly) {
		const resolvedCluster = clusterUrl
			? appHelper.getClusterValue(clusterUrl) || clusterUrl
			: appHelper.getDefaultClusterValue()
		appStore.activeClusterURL = resolvedCluster || null
	}
	await store.loadBuilder(pId, cId, clusterUrl)
})

onUnmounted(() => {
	store.selectedNode = undefined
	if (!authStore.isClusterReadonly && appHelper.getBaseURLs().length > 1) {
		appStore.activeClusterURL = null
	}
})
</script>

<template>
	<!-- Outer container matching Figma #1826:145088 (App background: #F8FAFC) -->
	<div
		class="h-[calc(100vh-64px)] w-full p-2 bg-[#F8FAFC] dark:bg-slate-950 flex flex-col overflow-hidden"
	>
		<!-- TopologyCanvas wrapper matching Figma #1826:145121 -->
		<div
			class="relative flex-1 w-full h-full bg-white dark:bg-slate-900 border border-[#D8D9DB] dark:border-slate-800 rounded-[16px] overflow-hidden flex flex-col"
		>
			<!-- Top Navigation Sub-Header Bar matching Figma #2031:94339 in node 1826:145087 -->
			<div class="absolute top-4 left-4 right-4 z-40">
				<CanvasTopNav />
			</div>

			<!-- Interactive Topology Canvas -->
			<main class="flex-1 w-full h-full relative overflow-hidden">
				<BuilderCanvas />
				<NodeConfigDrawer />
				<ProcessorGroupDrawer />
				<PreviewDataModal />
			</main>
		</div>
	</div>
</template>

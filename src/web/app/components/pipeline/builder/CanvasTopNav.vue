<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed } from 'vue'
import { useRouter } from 'vue-router'
import { Badge as UiBadge } from '@/components/ui/badge'
import { Button as UiButton } from '@/components/ui/button'
import UiIcon from '@/components/ui/icon/Icon.vue'
import { Separator as UiSeparator } from '@/components/ui/separator'
import HostSelectorPopover from '@/components/pipeline/HostSelectorPopover.vue'
import SinkSelectorPopover from '@/components/pipeline/SinkSelectorPopover.vue'
import ModalConfirmationDelete from '@/components/modal/ModalConfirmationDelete.vue'
import { usePipelineBuildStore } from '@/stores/pipeline.builder.store'
import type { IHostSelectorItem, ISinkSelectorItem } from '@@/shared/types/pipeline.type'

const router = useRouter()
const store = usePipelineBuildStore()

const pipelineName = computed(() => store.pipelineDetail?.name || 'Network Telemetry Pipeline')
const isRunning = computed(() => store.pipelineDetail?.adm_status === 1)

const hostCount = computed(() => store.pipelineDetail?.host_ids?.length || 0)
const nodeCounts = computed(() => {
	let processors = 0
	let sinks = 0
	for (const n of store.nodes) {
		if (n.type === 'processor') processors++
		else if (n.type === 'sink') sinks++
	}
	return { processors, sinks }
})
const isResourceActionDisabled = computed(() => !store.pipelineDetail || store.loading.canvas)

const mappedHosts = computed<IHostSelectorItem[]>(() =>
	(store.availableHosts || []).map((h: any) => ({
		id: h.host_id,
		name: h.name || h.host_id,
		ip: h.ip || h.endpoint || h.host_id,
		environment: (h.environment as any) || 'prod'
	}))
)

const mappedSinks = computed<ISinkSelectorItem[]>(() =>
	(store.availableSinks || []).map((s: any) => ({
		id: s.sink_id,
		name: s.name || s.sink_id,
		type: s.type || s.config?.type || 'sink'
	}))
)

const selectedHostIds = computed(() => store.pipelineDetail?.host_ids || [])
const selectedSinkIds = computed(() => store.pipelineDetail?.sink_ids || [])

function goBack() {
	router.push('/pipeline')
}
</script>

<template>
	<!-- navigation-sub-header-bar matching Figma #2031:94339 in node 1826:145087 -->
	<div
		class="w-full bg-white dark:bg-slate-900 border border-[#D8D9DB] dark:border-slate-800 rounded-[7.5px] px-5 py-2.5 flex items-center justify-between shadow-xs select-none min-h-[52px]"
	>
		<div class="flex items-center">
			<UiButton
				variant="ghost"
				size="sm"
				class="h-7 px-2 gap-1 text-xs text-foreground hover:text-primary transition-colors"
				@click="goBack"
			>
				<UiIcon name="arrow-left" size="16px" />
				<span>Back</span>
			</UiButton>

			<UiSeparator orientation="vertical" class="h-[15px] mx-3" />

			<div class="flex items-center gap-2.5">
				<h1
					class="text-[14.9px] font-bold text-[#0F172A] dark:text-slate-100 leading-none m-0"
				>
					{{ pipelineName }}
				</h1>

				<UiBadge
					:variant="isRunning ? 'secondary' : 'outline'"
					class="px-2 py-0.5 rounded-[8px] text-[10px] font-medium leading-[14px] transition-colors"
					:class="
						isRunning
							? 'border-transparent bg-emerald-100 text-emerald-700 hover:bg-emerald-100 dark:bg-emerald-950 dark:text-emerald-300'
							: 'border-transparent bg-[#FEF3C7] text-[#D97706] hover:bg-[#FEF3C7] dark:bg-amber-950 dark:text-amber-300'
					"
				>
					{{ isRunning ? 'Running' : 'Draft' }}
				</UiBadge>
			</div>
		</div>

		<div class="flex items-center gap-3.5">
			<span class="text-xs font-medium text-[#64748B] dark:text-slate-400">
				{{ hostCount }} Hosts &nbsp;·&nbsp; {{ nodeCounts.processors }} Processors
				&nbsp;·&nbsp; {{ nodeCounts.sinks }} Sink
			</span>

			<div class="flex items-center gap-2">
				<UiButton
					size="xs"
					class="gap-1 text-[10px]"
					title="Select hosts for this pipeline"
					:disabled="isResourceActionDisabled"
					@click="store.openHostSelectorModal()"
				>
					<UiIcon name="plus" size="14px" />
					<span>Select Hosts</span>
				</UiButton>

				<UiButton
					size="xs"
					class="gap-1 text-[10px]"
					title="Select sinks for this pipeline"
					:disabled="isResourceActionDisabled"
					@click="store.openSinkSelectorModal()"
				>
					<UiIcon name="plus" size="14px" />
					<span>Select Sinks</span>
				</UiButton>

				<UiButton
					size="xs"
					class="text-[10px]"
					:variant="store.isDirty ? 'default' : 'secondary'"
					:class="store.isDirty ? 'bg-amber-500 hover:bg-amber-600 text-white' : ''"
					:disabled="store.loading.saving || !store.isDirty"
					@click="store.saveGraph()"
				>
					<UiIcon
						v-if="store.loading.saving"
						name="loading"
						class="w-3 h-3 mr-1 animate-spin"
					/>
					<span>{{
						store.loading.saving
							? 'Saving...'
							: store.isDirty
								? 'Save Changes'
								: 'Saved'
					}}</span>
				</UiButton>
			</div>
		</div>
	</div>

	<HostSelectorPopover
		:model-value="store.hostSelectorModalOpen"
		:suspended="store.subsetConfirmation.isOpen"
		:hosts="mappedHosts"
		:selected-ids="selectedHostIds"
		:loading="store.loadingResources.hosts"
		:saving="store.loadingResources.updatingSubset"
		:min-selection="1"
		:error-message="store.selectorError"
		@apply="store.applyPipelineHosts"
		@clear-error="store.clearSelectorError"
		@close="store.closeHostSelectorModal"
		@update:model-value="(val) => !val && store.closeHostSelectorModal()"
	/>
	<SinkSelectorPopover
		:model-value="store.sinkSelectorModalOpen"
		:suspended="store.subsetConfirmation.isOpen"
		:sinks="mappedSinks"
		:selected-ids="selectedSinkIds"
		:loading="store.loadingResources.sinks"
		:saving="store.loadingResources.updatingSubset"
		:error-message="store.selectorError"
		@apply="store.applyPipelineSinks"
		@clear-error="store.clearSelectorError"
		@close="store.closeSinkSelectorModal"
		@update:model-value="(val) => !val && store.closeSinkSelectorModal()"
	/>

	<ModalConfirmationDelete
		title="Resources In Use"
		confirm-text="Remove & Apply"
		cancel-text="Cancel"
		:show-modal="store.subsetConfirmation.isOpen"
		:message="store.subsetConfirmation.message"
		:loading="store.loadingResources.updatingSubset"
		@dismiss="store.cancelSubsetConfirmation"
		@confirm="store.confirmSubsetForceUpdate"
	/>
</template>

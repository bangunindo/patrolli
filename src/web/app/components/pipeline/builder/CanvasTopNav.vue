<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed } from 'vue'
import { useRouter } from 'vue-router'
import { FormMode } from '@@/shared/types/app.type'
import { usePipelineBuildStore } from '@/stores/pipeline.builder.store'
import { useHostsStore } from '@/stores/hosts.store'
import { useSinksStore } from '@/stores/sinks.store'

import ModalHostForm from '@/components/configuration/host/ModalForm.vue'
import ModalSinkForm from '@/components/configuration/sinks/ModalForm.vue'

const router = useRouter()
const store = usePipelineBuildStore()
const hostsStore = useHostsStore()
const sinksStore = useSinksStore()

const pipelineName = computed(() => store.pipelineDetail?.name || 'Network Telemetry Pipeline')
const isRunning = computed(() => store.pipelineDetail?.adm_status === 1)

const hostCount = computed(() => store.pipelineDetail?.host_ids?.length || 0)
const processorCount = computed(() => store.nodes.filter((n) => n.type === 'processor').length)
const sinkCount = computed(() => store.nodes.filter((n) => n.type === 'sink').length)

function goBack() {
	router.push('/pipeline')
}

function handleAddHost() {
	hostsStore.toggleHostModalForm(true, FormMode.ADD)
}

function handleAddSink() {
	sinksStore.toggleSinkModalForm(true, FormMode.ADD)
}

async function handleSave() {
	await store.saveGraph()
}
</script>

<template>
	<!-- navigation-sub-header-bar matching Figma #2031:94339 in node 1826:145087 -->
	<div
		class="w-full bg-white dark:bg-slate-900 border border-[#D8D9DB] dark:border-slate-800 rounded-[7.5px] px-5 py-2.5 flex items-center justify-between shadow-xs select-none min-h-[52px]"
	>
		<!-- Left: Back Button, Divider, Pipeline Title & Status Badge -->
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

			<div class="w-[1px] h-[15px] bg-[#E2E8F0] dark:bg-slate-700 mx-3" />

			<div class="flex items-center gap-2.5">
				<h1
					class="text-[14.9px] font-bold text-[#0F172A] dark:text-slate-100 leading-none m-0"
				>
					{{ pipelineName }}
				</h1>

				<span
					class="px-2 py-0.5 rounded-[8px] text-[10px] font-medium leading-[14px] transition-colors"
					:class="
						isRunning
							? 'bg-emerald-100 text-emerald-700 dark:bg-emerald-950 dark:text-emerald-300'
							: 'bg-[#FEF3C7] text-[#D97706] dark:bg-amber-950 dark:text-amber-300'
					"
				>
					{{ isRunning ? 'Running' : 'Draft' }}
				</span>
			</div>
		</div>

		<!-- Right: Resources Counter & Action Buttons -->
		<div class="flex items-center gap-3.5">
			<span class="text-xs font-medium text-[#64748B] dark:text-slate-400">
				{{ hostCount }} Hosts &nbsp;·&nbsp; {{ processorCount }} Processors &nbsp;·&nbsp;
				{{ sinkCount }} Sink
			</span>

			<div class="flex items-center gap-2">
				<!-- Add Host Button -->
				<UiButton
					size="xs"
					class="gap-1 text-[10px]"
					title="Configure Hosts"
					@click="handleAddHost"
				>
					<UiIcon name="plus" size="14px" />
					<span>Add Host</span>
				</UiButton>

				<!-- Add Sink Button -->
				<UiButton
					size="xs"
					class="gap-1 text-[10px]"
					title="Add Sink Node"
					@click="handleAddSink"
				>
					<UiIcon name="plus" size="14px" />
					<span>Add Sink</span>
				</UiButton>

				<!-- Save / Publish Button -->
				<UiButton
					size="xs"
					class="text-[10px]"
					:variant="store.isDirty ? 'default' : 'secondary'"
					:class="store.isDirty ? 'bg-amber-500 hover:bg-amber-600 text-white' : ''"
					:disabled="store.loading.saving || !store.isDirty"
					@click="handleSave"
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

	<!-- Create Modal Forms -->
	<ModalHostForm v-if="hostsStore.hostModalShow" />
	<ModalSinkForm v-if="sinksStore.sinkModalShow" />
</template>

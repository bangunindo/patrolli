<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { ref, watch } from 'vue'
import AppInputCluster from '@/components/app/input/AppInputCluster.vue'
import AppInputText from '@/components/app/input/AppInputText.vue'
import HostSelectorPopover from '@/components/pipeline/HostSelectorPopover.vue'
import SinkSelectorPopover from '@/components/pipeline/SinkSelectorPopover.vue'
import { usePipelineStore } from '@/stores/pipeline.store'
import { FormMode } from '@@/shared/types/app.type'

const pipelineStore = usePipelineStore()
const validationError = ref('')
const showHostPopover = ref(false)
const showSinkPopover = ref(false)

function getHostName(hostId: string) {
	const found = pipelineStore.availableHosts.find((h) => h.id === hostId)
	return found?.name || hostId
}

function getSinkName(sinkId: string) {
	const found = pipelineStore.availableSinks.find((s) => s.id === sinkId)
	return found?.name || sinkId
}

function selectAccess(access: 'public' | 'private') {
	pipelineStore.form.access = access
	pipelineStore.form.visibility = access === 'public' ? 1 : 0
}

function removeHost(hostId: string) {
	const idx = pipelineStore.form.assignedHosts.indexOf(hostId)
	if (idx > -1) pipelineStore.form.assignedHosts.splice(idx, 1)
}

function removeSink(sinkId: string) {
	const idx = pipelineStore.form.assignedSinks.indexOf(sinkId)
	if (idx > -1) pipelineStore.form.assignedSinks.splice(idx, 1)
}

// Watch cluster change to reset hosts/sinks and fetch new lists
watch(
	() => pipelineStore.form.cluster_url,
	async (newCluster, oldCluster) => {
		if (newCluster && oldCluster && newCluster !== oldCluster) {
			pipelineStore.form.assignedHosts = []
			pipelineStore.form.assignedSinks = []
			await Promise.all([
				pipelineStore.fetchHostsForCluster(newCluster),
				pipelineStore.fetchSinksForCluster(newCluster)
			])
		}
	}
)

async function handleCreate() {
	validationError.value = ''
	if (!pipelineStore.form.name.trim()) {
		validationError.value = 'Pipeline Name is required.'
		return
	}
	if (pipelineStore.form.assignedHosts.length === 0) {
		validationError.value = 'At least one host must be assigned.'
		return
	}

	const success = await pipelineStore.savePipeline()
	if (!success && pipelineStore.error) {
		validationError.value = pipelineStore.error
	}
}
</script>

<template>
	<div class="flex-1 flex flex-col min-w-0 bg-white dark:bg-slate-900">
		<!-- Form Header matching Figma #2067:131427 -->
		<div
			class="flex items-center justify-between px-6 py-5 pr-12 border-b border-slate-200 dark:border-slate-800"
		>
			<div>
				<h3
					class="!mb-0 text-slate-900 dark:text-slate-100 font-semibold text-base leading-tight"
				>
					{{
						pipelineStore.pipelineFormMode === FormMode.UPDATE
							? 'Edit Pipeline'
							: 'Create New Pipeline'
					}}
				</h3>
				<p class="text-xs text-slate-500 dark:text-slate-400 mt-1 leading-normal">
					{{
						pipelineStore.pipelineFormMode === FormMode.UPDATE
							? 'Update your data pipeline settings and data sources.'
							: 'Configure your data pipeline settings and data sources.'
					}}
				</p>
			</div>
		</div>

		<!-- Form Body matching Figma #2067:131434 -->
		<div class="p-6 overflow-y-auto max-h-[72vh] flex flex-col gap-6">
			<!-- Validation Alert -->
			<div
				v-if="validationError"
				class="p-3 rounded-lg bg-rose-50 dark:bg-rose-950/50 border border-rose-200 dark:border-rose-900/80 text-rose-700 dark:text-rose-400 text-xs font-medium"
			>
				{{ validationError }}
			</div>

			<!-- SECTION 1: INFO -->
			<div class="flex flex-col gap-4">
				<div class="flex items-center gap-2">
					<div class="w-[3px] h-3.5 rounded-full bg-sky-600" />
					<span class="text-xs font-bold uppercase tracking-wider text-sky-600">
						SECTION 1: INFO
					</span>
				</div>

				<!-- Field: Select Cluster (Required) -->
				<div class="flex flex-col gap-1.5">
					<label class="text-xs font-semibold text-slate-800 dark:text-slate-200">
						Cluster <span class="text-[#DA1E28] font-bold">*</span>
					</label>
					<AppInputCluster
						v-model="pipelineStore.form.cluster_url"
						placeholder="Select Cluster"
						required
						:disabled="pipelineStore.pipelineFormMode === FormMode.UPDATE"
					/>
				</div>

				<!-- Field: Pipeline Name -->
				<div class="flex flex-col gap-1.5">
					<label class="text-xs font-semibold text-slate-800 dark:text-slate-200">
						Pipeline Name <span class="text-[#DA1E28] font-bold">*</span>
					</label>
					<AppInputText
						v-model="pipelineStore.form.name"
						placeholder="enter pipeline name"
						required
					/>
				</div>

				<!-- Field: Pipeline Access -->
				<div class="flex flex-col gap-2">
					<div class="flex items-center gap-1.5">
						<label class="text-xs font-semibold text-slate-800 dark:text-slate-200">
							Pipeline Access <span class="text-[#DA1E28] font-bold">*</span>
						</label>
						<UiIcon name="information-outline" size="14px" class="text-slate-400" />
					</div>

					<div class="flex flex-col gap-3">
						<!-- Radio Public matching Figma -->
						<div
							class="flex items-start gap-2.5 cursor-pointer select-none"
							@click="selectAccess('public')"
						>
							<div
								class="w-4 h-4 mt-0.5 rounded-full border flex items-center justify-center shrink-0 transition-colors"
								:class="
									pipelineStore.form.access === 'public'
										? 'border-sky-600'
										: 'border-slate-300 dark:border-slate-600'
								"
							>
								<div
									v-if="pipelineStore.form.access === 'public'"
									class="w-2 h-2 rounded-full bg-sky-600"
								/>
							</div>
							<div class="flex flex-col">
								<span
									class="text-xs font-medium text-slate-900 dark:text-slate-100"
								>
									Public
								</span>
								<span class="text-xs text-slate-400 dark:text-slate-500">
									Accessible to all team members
								</span>
							</div>
						</div>

						<!-- Radio Private matching Figma -->
						<div
							class="flex items-start gap-2.5 cursor-pointer select-none"
							@click="selectAccess('private')"
						>
							<div
								class="w-4 h-4 mt-0.5 rounded-full border flex items-center justify-center shrink-0 transition-colors"
								:class="
									pipelineStore.form.access === 'private'
										? 'border-sky-600'
										: 'border-slate-300 dark:border-slate-600'
								"
							>
								<div
									v-if="pipelineStore.form.access === 'private'"
									class="w-2 h-2 rounded-full bg-sky-600"
								/>
							</div>
							<div class="flex flex-col">
								<span
									class="text-xs font-medium text-slate-900 dark:text-slate-100"
								>
									Private
								</span>
								<span class="text-xs text-slate-400 dark:text-slate-500">
									Only visible to you and workspace administrators
								</span>
							</div>
						</div>
					</div>
				</div>
			</div>

			<!-- SECTION 2: SOURCES -->
			<div class="flex flex-col gap-4">
				<div class="flex items-center gap-2">
					<div class="w-[3px] h-3.5 rounded-full bg-sky-600" />
					<span class="text-xs font-bold uppercase tracking-wider text-sky-600">
						SECTION 2: SOURCES
					</span>
				</div>

				<div class="flex items-center justify-between">
					<div class="flex flex-col">
						<span class="text-xs font-semibold text-slate-800 dark:text-slate-200">
							Assign Hosts <span class="text-[#DA1E28] font-bold">*</span>
						</span>
						<span class="text-xs text-slate-500 dark:text-slate-400">
							Select the host environments that will send telemetry data
						</span>
					</div>

					<button
						type="button"
						class="flex items-center gap-1.5 px-3 py-1.5 rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-900 hover:bg-slate-50 dark:hover:bg-slate-800 text-xs font-medium text-slate-700 dark:text-slate-300 transition-colors shadow-xs"
						@click="showHostPopover = !showHostPopover"
					>
						<UiIcon name="plus" size="14px" />
						<span>Select Hosts</span>
					</button>
				</div>

				<!-- Host Selector Popover Card -->
				<HostSelectorPopover
					v-if="showHostPopover"
					:inline="true"
					@close="showHostPopover = false"
				/>

				<!-- Selected Hosts Control Box matching Figma #2067:131464 -->
				<div
					class="p-3 rounded-lg border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-900 flex flex-col gap-2.5 shadow-xs"
				>
					<div class="flex items-center justify-between">
						<span class="text-xs font-semibold text-slate-500 dark:text-slate-400">
							{{ pipelineStore.form.assignedHosts.length }} Hosts Selected
						</span>
					</div>

					<div class="h-px bg-slate-200 dark:bg-slate-800 w-full" />

					<!-- Selected hosts tags matching Figma #2067:131471 -->
					<div class="flex flex-wrap items-center gap-1.5">
						<div
							v-for="hostId in pipelineStore.form.assignedHosts"
							:key="hostId"
							class="inline-flex items-center gap-1.5 px-2 py-0.5 rounded bg-[#E0F2FE] dark:bg-sky-950/80 border border-sky-400 dark:border-sky-600 text-sky-700 dark:text-sky-300 text-xs font-normal"
						>
							<span>{{ getHostName(hostId) }}</span>
							<button
								type="button"
								class="text-sky-600 dark:text-sky-400 hover:text-red-500 font-bold text-xs leading-none"
								@click="removeHost(hostId)"
							>
								✕
							</button>
						</div>

						<span
							v-if="pipelineStore.form.assignedHosts.length === 0"
							class="text-xs text-slate-400 italic py-0.5"
						>
							No hosts assigned yet
						</span>
					</div>
				</div>
			</div>

			<!-- SECTION 3: SINKS -->
			<div class="flex flex-col gap-4">
				<div class="flex items-center gap-2">
					<div class="w-[3px] h-3.5 rounded-full bg-sky-600" />
					<span class="text-xs font-bold uppercase tracking-wider text-sky-600">
						SECTION 3: SINKS
					</span>
				</div>

				<div class="flex items-center justify-between">
					<div class="flex flex-col">
						<span class="text-xs font-semibold text-slate-800 dark:text-slate-200">
							Assign Sinks
						</span>
						<span class="text-xs text-slate-500 dark:text-slate-400">
							Select destinations where telemetry data will be routed
						</span>
					</div>

					<button
						type="button"
						class="flex items-center gap-1.5 px-3 py-1.5 rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-900 hover:bg-slate-50 dark:hover:bg-slate-800 text-xs font-medium text-slate-700 dark:text-slate-300 transition-colors shadow-xs"
						@click="showSinkPopover = !showSinkPopover"
					>
						<UiIcon name="plus" size="14px" />
						<span>Select Sinks</span>
					</button>
				</div>

				<!-- Sink Selector Popover Card -->
				<SinkSelectorPopover
					v-if="showSinkPopover"
					:inline="true"
					@close="showSinkPopover = false"
				/>

				<!-- Selected Sinks Control Box matching Figma #2068:9241 -->
				<div
					class="p-3 rounded-lg border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-900 flex flex-col gap-2.5 shadow-xs"
				>
					<div class="flex items-center justify-between">
						<span class="text-xs font-semibold text-slate-500 dark:text-slate-400">
							{{ pipelineStore.form.assignedSinks.length }} Sinks Selected
						</span>
					</div>

					<div class="h-px bg-slate-200 dark:bg-slate-800 w-full" />

					<!-- Selected sinks tags matching Figma #2068:9259 -->
					<div class="flex flex-wrap items-center gap-1.5">
						<div
							v-for="sinkId in pipelineStore.form.assignedSinks"
							:key="sinkId"
							class="inline-flex items-center gap-1.5 px-2 py-0.5 rounded bg-[#E0F2FE] dark:bg-sky-950/80 border border-sky-400 dark:border-sky-600 text-sky-700 dark:text-sky-300 text-xs font-normal"
						>
							<span>{{ getSinkName(sinkId) }}</span>
							<button
								type="button"
								class="text-sky-600 dark:text-sky-400 hover:text-red-500 font-bold text-xs leading-none"
								@click="removeSink(sinkId)"
							>
								✕
							</button>
						</div>

						<span
							v-if="pipelineStore.form.assignedSinks.length === 0"
							class="text-xs text-slate-400 italic py-0.5"
						>
							No sinks assigned (optional)
						</span>
					</div>
				</div>

				<span class="text-xs text-slate-500 dark:text-slate-400 font-normal">
					Optional - you can add sinks later
				</span>
			</div>
		</div>

		<!-- Form Footer matching Figma #2067:131477 -->
		<div
			class="flex items-center justify-end gap-3 px-6 py-4 border-t border-slate-200 dark:border-slate-800 bg-white dark:bg-slate-900"
		>
			<button
				type="button"
				class="px-4 py-2 text-xs font-medium rounded border border-slate-300 dark:border-slate-700 text-slate-800 dark:text-slate-200 hover:bg-slate-50 dark:hover:bg-slate-800 transition-colors"
				@click="pipelineStore.togglePipelineModalForm(false)"
			>
				Cancel
			</button>

			<button
				type="button"
				class="px-4 py-2 text-xs font-medium rounded bg-sky-600 hover:bg-sky-700 text-white transition-colors shadow-xs"
				:disabled="pipelineStore.loading.form"
				@click="handleCreate"
			>
				<span v-if="pipelineStore.loading.form">Saving...</span>
				<span v-else>
					{{
						pipelineStore.pipelineFormMode === FormMode.UPDATE
							? 'Save Changes'
							: 'Create Pipeline'
					}}
				</span>
			</button>
		</div>
	</div>
</template>

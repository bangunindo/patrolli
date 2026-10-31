<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import RocketIcon from '@/components/pipeline/RocketIcon.vue'
import { usePipelineStore } from '@/stores/pipeline.store'
import { FormMode } from '@@/shared/types/app.type'

const pipelineStore = usePipelineStore()
</script>

<template>
	<div
		class="w-full md:w-[318px] shrink-0 p-6 flex flex-col justify-between gap-5 bg-slate-50 dark:bg-slate-950/70 border-b md:border-b-0 md:border-r border-slate-200 dark:border-slate-800"
	>
		<div class="flex flex-col gap-5">
			<!-- Sidebar Header (centered rocket + title + description matching Figma #2067:131394) -->
			<div class="flex flex-col items-center text-center gap-2">
				<RocketIcon />
				<h3
					class="text-slate-900 dark:text-slate-100 font-semibold text-base leading-tight"
				>
					{{
						pipelineStore.pipelineFormMode === FormMode.UPDATE
							? 'Edit Your Pipeline'
							: "Let's Build Your Pipeline"
					}}
				</h3>
				<p class="text-xs text-slate-400 dark:text-slate-400 leading-relaxed max-w-[260px]">
					{{
						pipelineStore.pipelineFormMode === FormMode.UPDATE
							? 'Review and update the required configuration below.'
							: "You're just a few steps away. Complete the required setup below to get started."
					}}
				</p>
			</div>

			<!-- Checklist Items matching Figma #2067:131400 -->
			<div class="flex flex-col gap-3">
				<!-- Step 1: Info -->
				<div
					class="p-3.5 rounded-lg border bg-white dark:bg-slate-900 border-slate-200 dark:border-slate-800 flex flex-col gap-2 transition-all shadow-xs"
				>
					<div class="flex items-center justify-between">
						<div class="flex items-center gap-2">
							<span
								class="w-6 h-6 rounded-full flex items-center justify-center text-xs font-semibold"
								:class="
									pipelineStore.isStep1Completed
										? 'bg-sky-600 text-white'
										: 'bg-slate-200 dark:bg-slate-800 text-slate-500 dark:text-slate-400'
								"
							>
								1
							</span>
							<span class="text-xs font-semibold text-slate-900 dark:text-slate-100">
								Info
							</span>
							<span class="text-xs text-slate-400 dark:text-slate-500">
								(Name & Access)
							</span>
						</div>

						<span
							class="px-2 py-0.5 rounded-lg text-[10px] font-medium leading-tight"
							:class="
								pipelineStore.isStep1Completed
									? 'bg-[#EBFAF0] dark:bg-emerald-950/50 text-[#3BCE71] dark:text-emerald-400'
									: 'bg-[#F1F5F9] dark:bg-slate-800 text-[#94A3B8] dark:text-slate-400'
							"
						>
							{{ pipelineStore.isStep1Completed ? 'Completed' : 'Incomplete' }}
						</span>
					</div>
					<span class="text-[10px] font-bold tracking-wider text-[#DA1E28]">
						MANDATORY
					</span>
				</div>

				<!-- Step 2: Source -->
				<div
					class="p-3.5 rounded-lg border bg-white dark:bg-slate-900 border-slate-200 dark:border-slate-800 flex flex-col gap-2 transition-all shadow-xs"
				>
					<div class="flex items-center justify-between">
						<div class="flex items-center gap-2">
							<span
								class="w-6 h-6 rounded-full flex items-center justify-center text-xs font-semibold"
								:class="
									pipelineStore.isStep2Completed
										? 'bg-sky-600 text-white'
										: 'bg-slate-200 dark:bg-slate-800 text-slate-500 dark:text-slate-400'
								"
							>
								2
							</span>
							<span class="text-xs font-semibold text-slate-900 dark:text-slate-100">
								Source
							</span>
						</div>

						<span
							class="px-2 py-0.5 rounded-lg text-[10px] font-medium leading-tight"
							:class="
								pipelineStore.isStep2Completed
									? 'bg-[#EBFAF0] dark:bg-emerald-950/50 text-[#3BCE71] dark:text-emerald-400'
									: 'bg-[#F1F5F9] dark:bg-slate-800 text-[#94A3B8] dark:text-slate-400'
							"
						>
							{{ pipelineStore.isStep2Completed ? 'Completed' : 'Incomplete' }}
						</span>
					</div>
					<span class="text-[10px] font-bold tracking-wider text-[#DA1E28]">
						MANDATORY
					</span>
				</div>

				<!-- Step 3: Sink -->
				<div
					class="p-3.5 rounded-lg border bg-white dark:bg-slate-900 border-slate-200 dark:border-slate-800 flex flex-col gap-2 transition-all shadow-xs"
				>
					<div class="flex items-center justify-between">
						<div class="flex items-center gap-2">
							<span
								class="w-6 h-6 rounded-full flex items-center justify-center text-xs font-semibold"
								:class="
									pipelineStore.isStep3Completed
										? 'bg-sky-600 text-white'
										: 'bg-slate-200 dark:bg-slate-800 text-slate-500 dark:text-slate-400'
								"
							>
								3
							</span>
							<span class="text-xs font-semibold text-slate-900 dark:text-slate-100">
								Sink
							</span>
						</div>

						<span
							class="px-2 py-0.5 rounded-lg text-[10px] font-medium leading-tight"
							:class="
								pipelineStore.isStep3Completed
									? 'bg-[#EBFAF0] dark:bg-emerald-950/50 text-[#3BCE71] dark:text-emerald-400'
									: 'bg-[#F1F5F9] dark:bg-slate-800 text-[#94A3B8] dark:text-slate-400'
							"
						>
							{{ pipelineStore.isStep3Completed ? 'Completed' : 'Incomplete' }}
						</span>
					</div>
					<span
						class="text-[10px] font-bold tracking-wider text-[#616468] dark:text-slate-400"
					>
						OPTIONAL
					</span>
				</div>
			</div>
		</div>

		<!-- Progress Bar Container matching Figma #2067:131423 -->
		<div class="flex flex-col gap-2 pt-2">
			<div class="w-full h-1.5 rounded-full bg-slate-200 dark:bg-slate-800 overflow-hidden">
				<div
					class="h-full bg-sky-600 rounded-full transition-all duration-300"
					:style="{ width: `${(pipelineStore.completedStepsCount / 3) * 100}%` }"
				/>
			</div>
			<p class="text-xs text-slate-400 dark:text-slate-500">
				{{ pipelineStore.completedStepsCount }} of 3 completed
			</p>
		</div>
	</div>
</template>

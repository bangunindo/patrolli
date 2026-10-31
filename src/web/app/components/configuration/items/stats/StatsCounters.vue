<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { formatUtcDateTimeWithRelative } from '@/app/helpers/date.helper'
import { computed } from 'vue'

defineOptions({
	name: 'stats-counters',
	alias: 'StatsCounters'
})

interface Props {
	totalChecks?: number
	totalRecords?: number
	totalFailures?: number
	totalDropped?: number
	since?: string | null
	isListener?: boolean
}

const props = withDefaults(defineProps<Props>(), {
	totalChecks: 0,
	totalRecords: 0,
	totalFailures: 0,
	totalDropped: 0,
	since: null,
	isListener: false
})

const totalChecksFormatted = computed(() => props.totalChecks.toLocaleString())
const totalRecordsFormatted = computed(() => props.totalRecords.toLocaleString())
const totalFailuresFormatted = computed(() => props.totalFailures.toLocaleString())
const totalDroppedFormatted = computed(() => props.totalDropped.toLocaleString())

const sinceFormatted = computed(() => {
	if (!props.since) return null
	return formatUtcDateTimeWithRelative(props.since, 'dd MMM yyyy, HH:mm:ss', null as any)
})
</script>

<template>
	<div class="flex flex-col gap-2">
		<div class="flex items-center justify-between">
			<span
				class="text-[11px] font-bold tracking-wider uppercase text-slate-500 dark:text-slate-400"
			>
				COLLECTION COUNTERS
			</span>
			<span
				v-if="sinceFormatted"
				class="text-[10px] text-slate-400 font-mono"
				:title="`Counters started at ${sinceFormatted}. Node restarts reset these counters.`"
			>
				Since {{ sinceFormatted }}
			</span>
		</div>

		<div class="grid grid-cols-2 gap-2.5">
			<!-- Total Checks -->
			<div
				class="border border-slate-200/80 dark:border-slate-800 bg-white dark:bg-slate-900/50 rounded-2xl p-3 flex flex-col gap-0.5 shadow-sm min-w-0"
			>
				<span class="text-[11px] font-medium text-slate-500 dark:text-slate-400">
					{{ props.isListener ? 'Events' : 'Checks' }}
				</span>
				<span
					class="text-lg sm:text-xl font-bold text-slate-800 dark:text-slate-100 truncate"
					:title="totalChecksFormatted"
				>
					{{ totalChecksFormatted }}
				</span>
				<span class="text-[10px] text-slate-400">Total started</span>
			</div>

			<!-- Total Records -->
			<div
				class="border border-emerald-200/80 bg-emerald-50/60 dark:border-emerald-900/50 dark:bg-emerald-950/20 rounded-2xl p-3 flex flex-col gap-0.5 shadow-sm min-w-0"
			>
				<span class="text-[11px] font-medium text-emerald-700 dark:text-emerald-400">
					Records
				</span>
				<span
					class="text-lg sm:text-xl font-bold text-emerald-700 dark:text-emerald-400 truncate"
					:title="totalRecordsFormatted"
				>
					{{ totalRecordsFormatted }}
				</span>
				<span class="text-[10px] text-emerald-600/70 dark:text-emerald-400/60"
					>Rows collected</span
				>
			</div>

			<!-- Total Failures -->
			<div
				class="border border-rose-200/80 bg-rose-50/60 dark:border-rose-900/50 dark:bg-rose-950/20 rounded-2xl p-3 flex flex-col gap-0.5 shadow-sm min-w-0"
			>
				<span class="text-[11px] font-medium text-rose-700 dark:text-rose-400">
					Failures
				</span>
				<span
					class="text-lg sm:text-xl font-bold text-rose-700 dark:text-rose-400 truncate"
					:title="totalFailuresFormatted"
				>
					{{ totalFailuresFormatted }}
				</span>
				<span class="text-[10px] text-rose-600/70 dark:text-rose-400/60">Failed runs</span>
			</div>

			<!-- Dropped Records (RAW count, never %) -->
			<div
				class="border border-purple-200/80 bg-purple-50/60 dark:border-purple-900/50 dark:bg-purple-950/20 rounded-2xl p-3 flex flex-col gap-0.5 shadow-sm min-w-0"
			>
				<span class="text-[11px] font-medium text-purple-700 dark:text-purple-400">
					Dropped
				</span>
				<span
					class="text-lg sm:text-xl font-bold text-purple-700 dark:text-purple-400 truncate"
					:title="totalDroppedFormatted"
				>
					{{ totalDroppedFormatted }}
				</span>
				<span class="text-[10px] text-purple-600/70 dark:text-purple-400/60"
					>Queue drops</span
				>
			</div>
		</div>
	</div>
</template>

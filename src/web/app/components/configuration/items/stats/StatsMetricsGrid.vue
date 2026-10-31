<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import {
	diffUtcInMinutes,
	formatUtcDate,
	formatUtcDateTimeWithRelative,
	formatUtcRelative
} from '@/app/helpers/date.helper'
import { computed } from 'vue'

defineOptions({
	name: 'stats-metrics-grid',
	alias: 'StatsMetricsGrid'
})

interface Props {
	lastSuccess?: string | null
	lastNonempty?: string | null
	totalChecks?: number
	totalFailures?: number
	lastDurationMs?: number | null
	lastLagMs?: number | null
	lastRecordCount?: number | null
	isListener?: boolean
	schedule?: string
	nextCheck?: string | null
}

const props = withDefaults(defineProps<Props>(), {
	lastSuccess: null,
	lastNonempty: null,
	totalChecks: 0,
	totalFailures: 0,
	lastDurationMs: null,
	lastLagMs: null,
	lastRecordCount: null,
	isListener: false,
	schedule: 'Every 30s',
	nextCheck: null
})

const lastSuccessFormattedDate = computed(() => {
	return formatUtcDate(props.lastSuccess, 'dd MMM yyyy, HH:mm', '—')
})

const lastSuccessFormatted = computed(() => {
	if (!props.lastSuccess) return ''
	const rel = formatUtcRelative(props.lastSuccess)
	return rel ? `(${rel})` : ''
})

const lastNonemptyFormatted = computed(() => {
	return formatUtcDateTimeWithRelative(props.lastNonempty, 'dd MMM yyyy, HH:mm', '')
})

const isSilentSuccess = computed(() => {
	if (!props.lastSuccess) return false
	if (!props.lastNonempty) return true
	return diffUtcInMinutes(props.lastSuccess, props.lastNonempty) > 10
})

const successRate = computed(() => {
	if (props.totalChecks > 0) {
		const rate = Math.max(
			0,
			((props.totalChecks - props.totalFailures) / props.totalChecks) * 100
		)
		return `${rate.toFixed(1)}%`
	}
	return '—'
})

const lastDurationFormatted = computed(() => {
	if (props.isListener) return '—'
	if (props.lastDurationMs !== null && props.lastDurationMs !== undefined) {
		return `${props.lastDurationMs} ms`
	}
	return '—'
})

const lastLagFormatted = computed(() => {
	if (props.isListener) return '—'
	if (props.lastLagMs !== null && props.lastLagMs !== undefined) {
		return `${props.lastLagMs} ms`
	}
	return '—'
})

const lastRecordCountFormatted = computed(() => {
	if (props.lastRecordCount !== null && props.lastRecordCount !== undefined) {
		return `${props.lastRecordCount.toLocaleString()} rows`
	}
	return '—'
})

const nextCheckFormatted = computed(() => {
	if (!props.nextCheck) return 'Not scheduled'
	return formatUtcDateTimeWithRelative(props.nextCheck, 'HH:mm:ss', 'Not scheduled')
})
</script>

<template>
	<div class="grid grid-cols-2 gap-2.5 sm:gap-3">
		<!-- Metric 1: Last Successful Collection -->
		<div
			class="border border-slate-200/80 dark:border-slate-800 bg-white dark:bg-slate-900/50 rounded-2xl p-3 sm:p-3.5 flex flex-col justify-between shadow-sm min-w-0"
		>
			<span class="text-[10px] sm:text-[11px] font-medium text-slate-500 dark:text-slate-400">
				Last Successful Collection
			</span>
			<div class="mt-1">
				<div
					class="font-bold text-xs sm:text-sm text-slate-800 dark:text-slate-100 break-words leading-snug"
				>
					{{ lastSuccessFormattedDate }}
					<span
						v-if="lastSuccessFormatted"
						class="block text-[10px] sm:text-[11px] font-normal text-slate-400 mt-0.5"
					>
						{{ lastSuccessFormatted }}
					</span>
				</div>
				<div
					v-if="isSilentSuccess && lastNonemptyFormatted"
					class="text-[10px] text-amber-600 dark:text-amber-400 mt-0.5 break-words"
				>
					Silent (Last data: {{ lastNonemptyFormatted }})
				</div>
			</div>
		</div>

		<!-- Metric 2: Success Rate -->
		<div
			class="border border-slate-200/80 dark:border-slate-800 bg-white dark:bg-slate-900/50 rounded-2xl p-3 sm:p-3.5 flex flex-col justify-between shadow-sm min-w-0"
		>
			<span class="text-[10px] sm:text-[11px] font-medium text-slate-500 dark:text-slate-400">
				Success Rate
			</span>
			<div class="mt-1 flex items-baseline gap-1.5 flex-wrap">
				<span class="font-bold text-base sm:text-lg text-slate-800 dark:text-slate-100">
					{{ successRate }}
				</span>
				<span v-if="successRate !== '—'" class="text-[10px] text-slate-400"> overall </span>
			</div>
		</div>

		<!-- Metric 3: Latency & Execution -->
		<div
			class="border border-slate-200/80 dark:border-slate-800 bg-white dark:bg-slate-900/50 rounded-2xl p-3 sm:p-3.5 flex flex-col justify-between shadow-sm min-w-0"
		>
			<span class="text-[10px] sm:text-[11px] font-medium text-slate-500 dark:text-slate-400">
				{{ props.isListener ? 'Last Batch Size' : 'Last Duration / Lag' }}
			</span>
			<div class="mt-1">
				<template v-if="props.isListener">
					<div
						class="font-bold text-xs sm:text-sm text-slate-800 dark:text-slate-100 break-words"
					>
						{{ lastRecordCountFormatted }}
					</div>
				</template>
				<template v-else>
					<div
						class="font-bold text-xs sm:text-sm text-slate-800 dark:text-slate-100 break-words"
					>
						{{ lastDurationFormatted }}
					</div>
					<div class="text-[10px] text-slate-400 mt-0.5 break-words">
						Scheduler lag: {{ lastLagFormatted }}
					</div>
				</template>
			</div>
		</div>

		<!-- Metric 4: Frequency & Next Run -->
		<div
			class="border border-slate-200/80 dark:border-slate-800 bg-white dark:bg-slate-900/50 rounded-2xl p-3 sm:p-3.5 flex flex-col justify-between shadow-sm min-w-0"
		>
			<span class="text-[10px] sm:text-[11px] font-medium text-slate-500 dark:text-slate-400">
				{{ props.isListener ? 'Operation Mode' : 'Schedule & Next Run' }}
			</span>
			<div class="mt-1">
				<div
					class="font-bold text-xs sm:text-sm text-slate-800 dark:text-slate-100 break-words"
				>
					{{ props.schedule }}
				</div>
				<div v-if="!props.isListener" class="text-[10px] text-slate-400 mt-0.5 break-words">
					Next: {{ nextCheckFormatted }}
				</div>
			</div>
		</div>
	</div>
</template>

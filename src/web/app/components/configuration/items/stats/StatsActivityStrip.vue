<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { addUtcMinutes, formatUtcDate, parseUtcDate } from '@/app/helpers/date.helper'
import { parse_activity_window, type ActivityState } from '@/app/helpers/itemstats.helper'
import type { ActivityBucket } from '@@/shared/types/item.type'
import { computed, ref } from 'vue'

defineOptions({
	name: 'stats-activity-strip',
	alias: 'StatsActivityStrip'
})

const BUCKET_STATE_MAP: Record<
	ActivityState,
	{ cellClass: string; dotClass: string; label: string; bgClass: string }
> = {
	Ok: {
		bgClass: 'bg-emerald-500',
		cellClass: 'bg-emerald-500 hover:bg-emerald-400',
		dotClass: 'bg-emerald-400 dark:bg-emerald-600',
		label: 'Ok (Healthy)'
	},
	Unstable: {
		bgClass: 'bg-amber-500',
		cellClass: 'bg-amber-500 hover:bg-amber-400',
		dotClass: 'bg-amber-400 dark:bg-amber-600',
		label: 'Unstable (Ok + Failure)'
	},
	Failure: {
		bgClass: 'bg-rose-500',
		cellClass: 'bg-rose-500 hover:bg-rose-400',
		dotClass: 'bg-rose-400 dark:bg-rose-600',
		label: 'Failure'
	},
	Unrecorded: {
		bgClass: 'bg-slate-200 dark:bg-slate-800',
		cellClass: 'bg-slate-200 dark:bg-slate-800 hover:bg-slate-300 dark:hover:bg-slate-700',
		dotClass: 'bg-slate-400',
		label: 'Unrecorded'
	}
}

interface Props {
	activityOk?: number[]
	activityFail?: number[]
	activityWindowEnd?: string | null
}

const props = withDefaults(defineProps<Props>(), {
	activityOk: () => [],
	activityFail: () => [],
	activityWindowEnd: null
})

const activityBuckets = computed<ActivityBucket[]>(() => {
	const okArray = props.activityOk || []
	const failArray = props.activityFail || []
	const endDate = parseUtcDate(props.activityWindowEnd) || new Date()

	const parsed = parse_activity_window(okArray, failArray, endDate)

	return parsed.map((item, index) => {
		const start = item.time
		const end = addUtcMinutes(start, 5) || start
		return {
			step: index,
			time: item.time,
			state: item.state,
			startTime: formatUtcDate(start, 'HH:mm'),
			endTime: formatUtcDate(end, 'HH:mm'),
			timeLabel: `${formatUtcDate(start, 'dd MMM HH:mm')} – ${formatUtcDate(end, 'HH:mm')}`
		}
	})
})

const isExpanded = ref(false)

const activityRows = computed(() => {
	const labels = [
		{ start: '-24h', end: '-18h' },
		{ start: '-18h', end: '-12h' },
		{ start: '-12h', end: '-6h' },
		{ start: '-6h', end: 'Now' }
	]

	const buckets = activityBuckets.value
	const chunkSize = 72 // 6 hours * 12 buckets/hour = 72 buckets

	return labels.map((label, i) => ({
		startLabel: label.start,
		endLabel: label.end,
		buckets: buckets.slice(i * chunkSize, (i + 1) * chunkSize)
	}))
})

const activitySummary = computed(() => {
	let ok = 0
	let failure = 0
	let unstable = 0
	let unrecorded = 0

	for (const b of activityBuckets.value) {
		if (b.state === 'Ok') ok++
		else if (b.state === 'Failure') failure++
		else if (b.state === 'Unstable') unstable++
		else unrecorded++
	}

	return { ok, failure, unstable, unrecorded }
})

const hoveredBucket = ref<ActivityBucket | null>(null)
</script>

<template>
	<div class="flex flex-col gap-2">
		<div class="flex items-center justify-between">
			<div class="flex items-center gap-2">
				<span
					class="text-[11px] font-bold tracking-wider uppercase text-slate-500 dark:text-slate-400"
				>
					24-HOUR ACTIVITY (288 × 5-MIN BUCKETS)
				</span>
			</div>
		</div>

		<div class="flex items-center justify-between gap-1 flex-wrap">
			<div
				class="flex items-center gap-x-2.5 gap-y-1 flex-wrap text-[10px] text-slate-400 font-medium"
			>
				<span class="flex items-center gap-1">
					<span class="w-2 h-2 rounded-full bg-emerald-500 inline-block" /> OK
				</span>
				<span class="flex items-center gap-1">
					<span class="w-2 h-2 rounded-full bg-amber-500 inline-block" /> Unstable
				</span>
				<span class="flex items-center gap-1">
					<span class="w-2 h-2 rounded-full bg-rose-500 inline-block" /> Failure
				</span>
				<span class="flex items-center gap-1">
					<span
						class="w-2 h-2 rounded-full bg-slate-300 dark:bg-slate-700 inline-block"
					/>
					Unrecorded
				</span>
			</div>

			<div>
				<UiButton
					size="sm"
					variant="ghost"
					class="text-sky-600 hover:text-sky-700 h-7 text-xs flex items-center gap-1"
					@click="isExpanded = !isExpanded"
				>
					<span>{{ isExpanded ? 'Collapse' : 'Expand (6h)' }}</span>
					<UiIcon
						:name="isExpanded ? 'chevron-up' : 'chevron-down'"
						class="w-3.5 h-3.5"
					/>
				</UiButton>
			</div>
		</div>

		<div
			class="border border-slate-200/80 dark:border-slate-800 bg-white dark:bg-slate-900/50 rounded-2xl p-3 sm:p-4 flex flex-col gap-3 shadow-sm relative min-w-0"
		>
			<!-- Expanded Mode: 4 rows x 6-hour segments -->
			<div v-if="isExpanded" class="flex flex-col gap-2.5">
				<div v-for="(row, rIdx) in activityRows" :key="rIdx" class="flex flex-col gap-1">
					<div
						class="grid grid-cols-[repeat(72,1fr)] gap-[1.5px] p-1.5 bg-slate-50 dark:bg-slate-950/60 rounded-lg border border-slate-100 dark:border-slate-800/80"
						@mouseleave="hoveredBucket = null"
					>
						<div
							v-for="bucket in row.buckets"
							:key="bucket.step"
							class="h-5 rounded-[1.5px] transition-all cursor-pointer hover:scale-125 hover:z-10 hover:shadow-sm"
							:class="BUCKET_STATE_MAP[bucket.state]?.cellClass"
							@mouseenter="hoveredBucket = bucket"
						/>
					</div>

					<div
						class="flex justify-between items-center text-[10px] font-mono text-slate-400 px-1"
					>
						<span>{{ row.startLabel }}</span>
						<span>{{ row.endLabel }}</span>
					</div>
				</div>
			</div>

			<!-- Compact Mode: 1 Single Seamless Continuous Row (No hover, no separator) -->
			<div v-else class="flex flex-col gap-1.5">
				<div
					class="grid grid-cols-[repeat(288,1fr)] gap-0 h-6 overflow-hidden rounded-md bg-slate-50 dark:bg-slate-950/60 border border-slate-100 dark:border-slate-800/80"
				>
					<div
						v-for="bucket in activityBuckets"
						:key="bucket.step"
						class="h-full"
						:class="BUCKET_STATE_MAP[bucket.state]?.bgClass"
					/>
				</div>

				<!-- Strip Time Range Labels (Divided by 6h intervals across 24h) -->
				<div
					class="flex justify-between items-center text-[10px] font-mono text-slate-400 px-0.5"
				>
					<span>-24h</span>
					<span>-18h</span>
					<span>-12h</span>
					<span>-6h</span>
					<span>Now</span>
				</div>
			</div>

			<!-- Hover Tooltip (shown in expanded mode) -->
			<div
				v-if="isExpanded && hoveredBucket"
				class="flex items-center justify-between px-3 py-1.5 rounded-lg bg-slate-800 text-white dark:bg-slate-100 dark:text-slate-900 text-xs font-mono shadow-md animate-in fade-in duration-150"
			>
				<span>{{ hoveredBucket.timeLabel }}</span>
				<span class="font-bold flex items-center gap-1.5">
					<span
						class="w-2 h-2 rounded-full"
						:class="BUCKET_STATE_MAP[hoveredBucket.state]?.dotClass"
					/>
					{{ BUCKET_STATE_MAP[hoveredBucket.state]?.label }}
				</span>
			</div>

			<!-- Activity Breakdown -->
			<div
				class="grid grid-cols-2 sm:grid-cols-4 gap-1.5 text-[11px] sm:text-xs pt-2 border-t border-slate-100 dark:border-slate-800 text-slate-500 dark:text-slate-400"
			>
				<span class="truncate">
					Ok:
					<strong class="text-emerald-600 dark:text-emerald-400">
						{{ activitySummary.ok }}
					</strong>
				</span>
				<span class="truncate">
					Unstable:
					<strong class="text-amber-600 dark:text-amber-400">
						{{ activitySummary.unstable }}
					</strong>
				</span>
				<span class="truncate">
					Failure:
					<strong class="text-rose-600 dark:text-rose-400">
						{{ activitySummary.failure }}
					</strong>
				</span>
				<span class="truncate">
					Unrecorded:
					<strong class="text-slate-600 dark:text-slate-300">
						{{ activitySummary.unrecorded }}
					</strong>
				</span>
			</div>
		</div>
	</div>
</template>

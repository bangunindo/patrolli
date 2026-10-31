<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { formatUtcRelative } from '@/app/helpers/date.helper'
import { FAILURE_KIND_META_MAP } from '@@/shared/constants/items.constant'
import type { Failure, FailureKind, ItemState } from '@@/shared/generated/patrolliAPI.schemas.type'
import type { FailureKindMeta } from '@@/shared/types/item.type'
import { computed } from 'vue'

defineOptions({
	name: 'stats-alert-banner',
	alias: 'StatsAlertBanner'
})

const props = withDefaults(
	defineProps<{
		itemState: ItemState
		lastFailure?: Failure | null
		consecutiveFailures?: number
		retriesSpent?: number
	}>(),
	{
		lastFailure: null,
		consecutiveFailures: 0,
		retriesSpent: 0
	}
)

const isActivelyFailing = computed(() => props.itemState === 'failing')
const isActivelyRetrying = computed(() => props.itemState === 'retrying')
const isActivelyStale = computed(() => props.itemState === 'stale')

const hasRecoveredFailure = computed(() => {
	return (
		(props.itemState === 'ok' || props.itemState === 'ok_empty') && Boolean(props.lastFailure)
	)
})

const activeFailureMeta = computed<FailureKindMeta>(() => {
	const kind = (props.lastFailure?.kind || 'unreachable') as FailureKind
	return (
		FAILURE_KIND_META_MAP[kind] || {
			label: kind,
			category: 'Target',
			hint: 'Check target host availability and configuration.',
			color: 'text-rose-600 dark:text-rose-400 bg-rose-50 dark:bg-rose-950/40 border-rose-200 dark:border-rose-900/50'
		}
	)
})
</script>

<template>
	<!-- Case A: Active Failure -->
	<div
		v-if="isActivelyFailing"
		class="border rounded-2xl p-3 sm:p-4 bg-rose-50/90 border-rose-200 dark:bg-rose-950/30 dark:border-rose-900/60 flex flex-col gap-2.5 shadow-sm min-w-0"
	>
		<div class="flex items-center justify-between gap-2 flex-wrap">
			<div class="flex items-center gap-2 text-rose-700 dark:text-rose-400 font-bold text-sm">
				<UiIcon name="alert-circle" class="w-4 h-4 shrink-0" />
				<span>Collection Failing</span>
			</div>
			<span
				v-if="props.consecutiveFailures > 0"
				class="text-[10px] font-bold px-2 py-0.5 rounded-md bg-rose-200/80 dark:bg-rose-900/60 text-rose-800 dark:text-rose-200 uppercase tracking-wide"
			>
				{{ props.consecutiveFailures }} consecutive
				{{ props.consecutiveFailures === 1 ? 'failure' : 'failures' }}
			</span>
		</div>

		<div class="flex items-center gap-2 text-xs flex-wrap">
			<span
				class="px-2 py-0.5 rounded border text-[10px] font-semibold"
				:class="activeFailureMeta.color"
			>
				{{ activeFailureMeta.label }}
			</span>
			<span class="text-[10px] text-slate-500 dark:text-slate-400">
				Origin: {{ activeFailureMeta.category }}
			</span>
		</div>

		<p
			class="text-xs text-rose-700/90 dark:text-rose-300 leading-relaxed font-mono bg-rose-100/50 dark:bg-rose-900/30 p-2 rounded-lg break-words break-all"
		>
			{{ props.lastFailure?.reason || 'Collection attempt failed without explicit reason.' }}
		</p>

		<div
			class="text-xs text-rose-800 dark:text-rose-300/90 flex items-start gap-1.5 leading-snug"
		>
			<UiIcon name="lightbulb-on-outline" class="w-4 h-4 shrink-0 mt-0.5 text-amber-500" />
			<span class="break-words">{{ activeFailureMeta.hint }}</span>
		</div>
	</div>

	<!-- Case B: Retrying State -->
	<div
		v-else-if="isActivelyRetrying"
		class="border rounded-2xl p-3 sm:p-4 bg-amber-50/90 border-amber-200 dark:bg-amber-950/30 dark:border-amber-900/60 flex flex-col gap-2 shadow-sm min-w-0"
	>
		<div class="flex items-center justify-between gap-2 flex-wrap">
			<div
				class="flex items-center gap-2 text-amber-700 dark:text-amber-400 font-bold text-sm"
			>
				<UiIcon name="refresh" class="w-4 h-4 shrink-0 animate-spin" />
				<span>Collection Retrying (Transient Issue)</span>
			</div>
			<span
				v-if="props.retriesSpent > 0"
				class="text-[10px] font-bold px-2 py-0.5 rounded-md bg-amber-200/80 dark:bg-amber-900/60 text-amber-800 dark:text-amber-200"
			>
				Attempt #{{ props.retriesSpent }}
			</span>
		</div>
		<p class="text-xs text-amber-800 dark:text-amber-300 leading-relaxed break-words break-all">
			{{
				props.lastFailure?.reason ||
				'The item encountered a temporary failure and is currently retrying.'
			}}
		</p>
		<p class="text-xs text-amber-700 dark:text-amber-400 break-words">
			&rarr; {{ activeFailureMeta.hint }}
		</p>
	</div>

	<!-- Case C: Stale State (Patrolli node issue, NOT target) -->
	<div
		v-else-if="isActivelyStale"
		class="border rounded-2xl p-3 sm:p-4 bg-sky-50/90 border-sky-200 dark:bg-sky-950/30 dark:border-sky-900/60 flex flex-col gap-2 shadow-sm min-w-0"
	>
		<div class="flex items-center gap-2 text-sky-700 dark:text-sky-400 font-bold text-sm">
			<UiIcon name="clock-alert-outline" class="w-4 h-4 shrink-0" />
			<span>Collection Stale (Scheduler Overdue)</span>
		</div>
		<p class="text-xs text-sky-800 dark:text-sky-300 leading-relaxed break-words">
			The Patrolli collector node has not executed this check on schedule. The target device
			is likely unaffected; this indicates node oversubscription or scheduler queue lag.
		</p>
	</div>

	<!-- Case D: Recovered notice (Subtle informational note, NOT alarm) -->
	<div
		v-else-if="hasRecoveredFailure"
		class="border rounded-xl px-3.5 py-2.5 bg-slate-50 dark:bg-slate-900/40 border-slate-200 dark:border-slate-800 flex items-center justify-between text-xs text-slate-600 dark:text-slate-400 gap-2 flex-wrap min-w-0"
	>
		<div class="flex items-center gap-2 min-w-0">
			<UiIcon name="check-circle-outline" class="w-4 h-4 text-emerald-500 shrink-0" />
			<span class="truncate">Item healthy (Recovered)</span>
		</div>
		<span class="text-[11px] text-slate-400 shrink-0">
			Last: {{ formatUtcRelative(props.lastFailure?.at) }}
		</span>
	</div>
</template>

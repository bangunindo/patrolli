<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { formatUtcDate, formatUtcRelative } from '@/app/helpers/date.helper'
import { FAILURE_KIND_META_MAP } from '@@/shared/constants/items.constant'
import type { Failure, FailureKind } from '@@/shared/generated/patrolliAPI.schemas.type'
import { computed, ref } from 'vue'

defineOptions({
	name: 'stats-recent-failures',
	alias: 'StatsRecentFailures'
})

interface Props {
	failures?: Failure[] | null
}

const props = withDefaults(defineProps<Props>(), {
	failures: () => []
})

const recentFailures = computed(() => {
	const list = props.failures
	if (Array.isArray(list) && list.length > 0) {
		return list.slice(0, 10).map((f, idx) => ({
			id: idx,
			at: formatUtcDate(f.at, 'dd MMM yyyy, HH:mm:ss', '—'),
			relativeAt: formatUtcRelative(f.at),
			kind: (f.kind || 'internal') as FailureKind,
			reason: f.reason || 'Unknown error',
			detail: f.detail ?? null,
			meta: FAILURE_KIND_META_MAP[(f.kind || 'internal') as FailureKind] || {
				label: f.kind,
				category: 'Target',
				hint: 'Check target host availability.',
				color: 'text-rose-600 bg-rose-50 border-rose-200'
			}
		}))
	}
	return []
})

const expandedErrors = ref<Record<number, boolean>>({})
function toggleErrorDetail(id: number) {
	expandedErrors.value[id] = !expandedErrors.value[id]
}
</script>

<template>
	<div class="flex flex-col gap-2">
		<div class="flex items-center justify-between">
			<div class="flex items-center gap-1.5">
				<span
					class="text-[11px] font-bold tracking-wider uppercase text-slate-500 dark:text-slate-400"
				>
					RECENT FAILURES
				</span>
				<span
					v-if="recentFailures.length > 0"
					class="text-[10px] font-bold px-1.5 py-0.2 rounded-full bg-slate-200 dark:bg-slate-800 text-slate-600 dark:text-slate-300"
				>
					{{ recentFailures.length }}
				</span>
			</div>
		</div>

		<!-- Empty State -->
		<div
			v-if="recentFailures.length === 0"
			class="border border-slate-200/80 dark:border-slate-800 bg-white dark:bg-slate-900/50 rounded-2xl p-6 flex flex-col items-center justify-center gap-1.5 text-center text-slate-400"
		>
			<UiIcon name="check-circle-outline" class="w-6 h-6 text-emerald-500 mb-1" />
			<span class="text-xs font-semibold text-slate-700 dark:text-slate-300">
				No Recent Failures
			</span>
			<span class="text-[11px]">
				All recent collection runs completed successfully without errors.
			</span>
		</div>

		<!-- Failures List -->
		<div v-else class="flex flex-col gap-2">
			<div
				v-for="err in recentFailures"
				:key="err.id"
				class="border border-slate-200/80 dark:border-slate-800 bg-white dark:bg-slate-900/50 rounded-xl p-3 flex flex-col gap-2 shadow-sm text-xs transition-all"
			>
				<div class="flex items-center justify-between gap-2">
					<div class="flex items-center gap-2 min-w-0">
						<span
							class="px-2 py-0.5 rounded border text-[10px] font-semibold shrink-0"
							:class="err.meta.color"
						>
							{{ err.meta.label }}
						</span>
						<span class="text-[11px] text-slate-400 font-mono truncate">
							{{ err.at }}
						</span>
					</div>
					<span class="text-[10px] text-slate-400 shrink-0">
						{{ err.relativeAt }}
					</span>
				</div>

				<p
					class="font-mono text-xs text-slate-700 dark:text-slate-300 bg-slate-50 dark:bg-slate-950/60 p-2 rounded-lg break-words break-all leading-relaxed"
				>
					{{ err.reason }}
				</p>

				<div
					class="flex items-center justify-between pt-1 border-t border-slate-100 dark:border-slate-800 text-[11px] gap-2"
				>
					<span class="text-slate-500 dark:text-slate-400 italic min-w-0 break-words">
						Hint: {{ err.meta.hint }}
					</span>

					<button
						v-if="err.detail"
						class="text-blue-600 dark:text-blue-400 hover:underline font-semibold shrink-0 ml-2"
						@click="toggleErrorDetail(err.id)"
					>
						{{ expandedErrors[err.id] ? 'Hide Details' : 'View Details' }}
					</button>
				</div>

				<!-- Collapsible detail payload viewer -->
				<div
					v-if="err.detail && expandedErrors[err.id]"
					class="mt-1 p-2.5 rounded-lg bg-slate-900 text-slate-200 dark:bg-black text-[11px] font-mono overflow-x-auto"
				>
					<pre class="whitespace-pre-wrap leading-tight">{{
						JSON.stringify(err.detail, null, 2)
					}}</pre>
				</div>
			</div>
		</div>
	</div>
</template>

<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { STATE_META_MAP } from '@@/shared/constants/items.constant'
import type { ItemState } from '@@/shared/generated/patrolliAPI.schemas.type'
import type { StateMeta } from '@@/shared/types/item.type'
import { computed } from 'vue'

defineOptions({
	name: 'stats-header',
	alias: 'StatsHeader'
})

const props = defineProps<{
	itemName: string
	itemId: string
	itemType: string
	itemState: ItemState
}>()

const stateMeta = computed<StateMeta>(() => {
	return STATE_META_MAP[props.itemState] || STATE_META_MAP.unknown
})
</script>

<template>
	<div
		class="border border-slate-200/80 dark:border-slate-800 bg-white dark:bg-slate-900/60 rounded-2xl p-3 sm:p-4 flex items-center justify-between gap-2.5 sm:gap-3 shadow-sm min-w-0"
	>
		<div class="flex items-center gap-2.5 sm:gap-3 min-w-0">
			<div
				class="w-9 h-9 sm:w-10 sm:h-10 rounded-xl bg-blue-50 dark:bg-blue-950/60 text-blue-600 dark:text-blue-400 flex items-center justify-center shrink-0 border border-blue-100 dark:border-blue-900/40"
			>
				<UiIcon name="database-outline" class="w-5 h-5" />
			</div>
			<div class="flex flex-col min-w-0">
				<span
					class="font-bold text-xs sm:text-sm truncate leading-tight"
					:title="props.itemName"
				>
					{{ props.itemName }}
				</span>
				<span
					class="text-[10px] sm:text-[11px] text-slate-400 font-mono mt-0.5 truncate"
					:title="props.itemType"
				>
					{{ props.itemType }}
				</span>
			</div>
		</div>

		<div class="flex flex-col items-end gap-1.5 shrink-0">
			<UiBadge variant="outline" class="uppercase font-semibold tracking-wider text-[10px]">
				{{ props.itemType }}
			</UiBadge>

			<!-- State Pill -->
			<UiBadge variant="outline" class="flex items-center gap-1">
				<span class="w-2 h-2 rounded-full shrink-0" :class="stateMeta.dotClass" />
				<span class="text-xs sm:text-sm">{{ stateMeta.label }}</span>
			</UiBadge>
		</div>
	</div>
</template>

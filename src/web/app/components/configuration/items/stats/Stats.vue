<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { formatSchedule } from '@/app/helpers/schedule.helper'
import { useItemsStore } from '@/stores/items.store'
import type {
	ItemsStatsResponse,
	ItemState,
	StatsSnapshot
} from '@@/shared/generated/patrolliAPI.schemas.type'
import type { IItem, IItems } from '@@/shared/types/item.type'
import { computed } from 'vue'

import StatsActivityStrip from '@/components/configuration/items/stats/StatsActivityStrip.vue'
import StatsAlertBanner from '@/components/configuration/items/stats/StatsAlertBanner.vue'
import StatsCounters from '@/components/configuration/items/stats/StatsCounters.vue'
import StatsHeader from '@/components/configuration/items/stats/StatsHeader.vue'
import StatsMetricsGrid from '@/components/configuration/items/stats/StatsMetricsGrid.vue'
import StatsRecentFailures from '@/components/configuration/items/stats/StatsRecentFailures.vue'

defineOptions({
	name: 'item-stats',
	alias: 'ItemStats'
})

const props = defineProps<{
	item?: IItem | IItems | null
}>()

const itemsStore = useItemsStore()

const currentItem = computed(() => {
	return itemsStore.currentItem || null
})

const statsResponse = computed<ItemsStatsResponse | null>(() => {
	return (currentItem.value as any)?.stats || null
})

const stats = computed<StatsSnapshot | null>(() => {
	return statsResponse.value?.snapshot || null
})

const since = computed(() => {
	return statsResponse.value?.since || null
})

// Item Info
const itemName = computed(() => {
	return currentItem.value?.resolved_name || currentItem.value?.name || 'Item Statistics'
})

const itemId = computed(() => {
	if (!currentItem.value) return '-'
	return currentItem.value.item_id || currentItem.value.name || '-'
})

const itemType = computed(() => {
	return (currentItem.value?.type || 'unknown').toLowerCase()
})

const isListenerItem = computed(() => {
	return itemType.value.includes('listener')
})

const itemState = computed<ItemState>(() => {
	if (!stats.value) return 'unknown'
	return stats.value.state || 'unknown'
})

const collectionSchedule = computed(() => {
	if (isListenerItem.value) return 'Push / Event Driven'
	if (currentItem.value?.resolved_schedule) {
		return formatSchedule(currentItem.value.resolved_schedule)
	}
	if ((currentItem.value as any)?.schedule) {
		return formatSchedule((currentItem.value as any).schedule)
	}
	return 'Every 30s'
})
</script>

<template>
	<div
		class="flex flex-col gap-3 sm:gap-4 w-full text-slate-800 dark:text-slate-100 pb-4 min-w-0"
	>
		<!-- 1. Header Card -->
		<StatsHeader
			:item-name="itemName"
			:item-id="itemId"
			:item-type="itemType"
			:item-state="itemState"
		/>

		<!-- 2. Active Alert / Status Banner -->
		<StatsAlertBanner
			:item-state="itemState"
			:last-failure="stats?.last_failure"
			:consecutive-failures="stats?.consecutive_failures"
			:retries-spent="stats?.retries"
		/>

		<!-- 3. Metrics 2x2 Grid -->
		<StatsMetricsGrid
			:last-success="stats?.last_success"
			:last-nonempty="stats?.last_nonempty"
			:total-checks="stats?.total_checks"
			:total-failures="stats?.total_failures"
			:last-duration-ms="stats?.last_duration_ms"
			:last-lag-ms="stats?.last_lag_ms"
			:last-record-count="stats?.last_record_count"
			:is-listener="isListenerItem"
			:schedule="collectionSchedule"
			:next-check="stats?.next_check"
		/>

		<!-- 4. Collection Counters (All since `since`) -->
		<StatsCounters
			:total-checks="stats?.total_checks"
			:total-records="stats?.total_records"
			:total-failures="stats?.total_failures"
			:total-dropped="stats?.total_dropped"
			:since="since"
			:is-listener="isListenerItem"
		/>

		<!-- 5. 24-Hour Activity Strip (288 × 5-min Buckets) -->
		<StatsActivityStrip
			:activity-ok="stats?.activity_ok"
			:activity-fail="stats?.activity_fail"
			:activity-window-end="stats?.activity_window_end"
		/>

		<!-- 6. Recent Failures (Up to 10 entries) -->
		<StatsRecentFailures :failures="stats?.recent_failures" />
	</div>
</template>

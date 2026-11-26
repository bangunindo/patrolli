<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import appHelper from '@/app/helpers/app.helper'
import AppFilterDropdown from '@/components/app/AppFilterDropdown.vue'
import AppInputAutocomplete from '@/components/app/input/AppInputAutocomplete.vue'
import SinkTable from '@/components/configuration/sinks/Table.vue'
import { useSinksStore } from '@/stores/sinks.store'
import type { SinkConfigType } from '@@/shared/types/sink.type'
import { computed, onUnmounted, reactive, watch } from 'vue'
import { useRoute } from 'vue-router'

definePageMeta({
	layout: 'configuration',
	requiresAuth: true
})

const route = useRoute()
const sinksStore = useSinksStore()

const getClusterUrls = () => appHelper.getClusterUrls(route.query.cluster as string)

const localFilter = reactive<{
	types: SinkConfigType[]
}>({
	types: sinksStore.filter.types ? [...sinksStore.filter.types] : []
})

const hasActiveFilter = computed(() => localFilter.types.length > 0)

function resetFilter() {
	localFilter.types = []
}

function applyFilter() {
	sinksStore.filter.types = localFilter.types.length > 0 ? [...localFilter.types] : undefined
	sinksStore.pagination.currentPage = 1
	sinksStore.filter.offset = 0
	sinksStore.fetchSinks(getClusterUrls())
}

watch(
	localFilter,
	() => {
		applyFilter()
	},
	{ deep: true }
)

onUnmounted(() => {
	sinksStore.resetFilter()
	sinksStore.currentSink = null
})
</script>

<template>
	<AppLayoutPageContainer>
		<AppLayoutPageHeader title="Sinks">
			<template #actions>
				<AppFilterDropdown :active="hasActiveFilter" @reset="resetFilter">
					<AppInputAutocomplete
						v-model="localFilter.types"
						label="Type"
						placeholder="All Types"
						:options="sinksStore.selection.types"
						multiple
						:portal="false"
						clearable
					/>
				</AppFilterDropdown>

				<UiButton @click="sinksStore.toggleSinkModalForm(true)">
					<UiIcon name="plus" size="16px" />
					<span>Create Sink</span>
				</UiButton>
			</template>
		</AppLayoutPageHeader>

		<sink-table />
	</AppLayoutPageContainer>
</template>

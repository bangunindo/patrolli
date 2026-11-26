<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import appHelper from '@/app/helpers/app.helper'
import AppFilterDropdown from '@/components/app/AppFilterDropdown.vue'
import AppInputAutocomplete from '@/components/app/input/AppInputAutocomplete.vue'
import ListenerTable from '@/components/configuration/listener/Table.vue'
import { useListenerStore } from '@/stores/listeners.store'
import type { ListenerType } from '@@/shared/types/listener.type'
import { computed, onUnmounted, reactive, watch } from 'vue'
import { useRoute } from 'vue-router'

definePageMeta({
	layout: 'configuration',
	requiresAuth: true
})

const route = useRoute()
const listenersStore = useListenerStore()

const getClusterUrls = () => appHelper.getClusterUrls(route.query.cluster as string)

const localFilter = reactive<{
	types: ListenerType[]
}>({
	types: listenersStore.filter.types ? [...listenersStore.filter.types] : []
})

const hasActiveFilter = computed(() => localFilter.types.length > 0)

function resetFilter() {
	localFilter.types = []
}

function applyFilter() {
	listenersStore.filter.types = localFilter.types.length > 0 ? [...localFilter.types] : undefined
	listenersStore.pagination.currentPage = 1
	listenersStore.filter.offset = 0
	listenersStore.fetchListeners(getClusterUrls())
}

watch(
	localFilter,
	() => {
		applyFilter()
	},
	{ deep: true }
)

onUnmounted(() => {
	listenersStore.resetFilter()
	listenersStore.currentListener = null
})
</script>

<template>
	<AppLayoutPageContainer>
		<AppLayoutPageHeader title="Listener">
			<template #actions>
				<AppFilterDropdown :active="hasActiveFilter" @reset="resetFilter">
					<AppInputAutocomplete
						v-model="localFilter.types"
						label="Type"
						placeholder="All Types"
						:options="listenersStore.selection.types"
						multiple
						:portal="false"
						clearable
					/>
				</AppFilterDropdown>

				<UiButton @click="listenersStore.toggleListenerModalForm(true)">
					<UiIcon name="plus" size="16px" />
					<span>Create Listener</span>
				</UiButton>
			</template>
		</AppLayoutPageHeader>

		<listener-table />
	</AppLayoutPageContainer>
</template>

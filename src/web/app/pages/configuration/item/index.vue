<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import appHelper from '@/app/helpers/app.helper'
import { formatUtcDateTimeWithRelative } from '@/app/helpers/date.helper'
import AppFilterDropdown from '@/components/app/AppFilterDropdown.vue'
import AppInputAutocomplete from '@/components/app/input/AppInputAutocomplete.vue'
import AppLoading from '@/components/app/loading/AppLoading.vue'
import ConfigDrawer from '@/components/app/navigation/ConfigDrawer.vue'
import Stats from '@/components/configuration/items/stats/Stats.vue'
import ItemsTable from '@/components/configuration/items/Table.vue'
import { useConfigurationStore } from '@/stores/configuration.store'
import { useItemsStore } from '@/stores/items.store'
import type { ItemType } from '@@/shared/types/item.type'
import { storeToRefs } from 'pinia'
import { computed, onMounted, onUnmounted, reactive, watch } from 'vue'
import { useRoute } from 'vue-router'

definePageMeta({
	layout: 'configuration',
	requiresAuth: true
})

const route = useRoute()
const itemsStore = useItemsStore()
const configurationStore = useConfigurationStore()

const getClusterUrls = (): string[] | undefined => {
	const clusterUrl = appHelper.getClusterValue(route.query.cluster as string)
	return clusterUrl ? [clusterUrl] : undefined
}

const localFilter = reactive<{
	types: ItemType[]
	endpoint_ids: string[]
	listener_ids: string[]
}>({
	types: itemsStore.filter.types ? [...itemsStore.filter.types] : [],
	endpoint_ids: itemsStore.filter.endpoint_ids ? [...itemsStore.filter.endpoint_ids] : [],
	listener_ids: itemsStore.filter.listener_ids ? [...itemsStore.filter.listener_ids] : []
})

onMounted(() => {
	const urls = getClusterUrls()
	itemsStore.fetchHosts(urls)
	itemsStore.fetchEndpoints(urls)
	itemsStore.fetchListeners(urls)
})

function onSearchEndpoints(query: string) {
	itemsStore.fetchEndpoints(getClusterUrls(), query)
}

function onSearchListeners(query: string) {
	itemsStore.fetchListeners(getClusterUrls(), query)
}

const hasActiveFilter = computed(
	() =>
		localFilter.types.length > 0 ||
		localFilter.endpoint_ids.length > 0 ||
		localFilter.listener_ids.length > 0
)

function resetFilter() {
	localFilter.types = []
	localFilter.endpoint_ids = []
	localFilter.listener_ids = []
}

function applyFilter() {
	itemsStore.filter.types = localFilter.types.length > 0 ? [...localFilter.types] : undefined
	itemsStore.filter.endpoint_ids =
		localFilter.endpoint_ids.length > 0 ? [...localFilter.endpoint_ids] : undefined
	itemsStore.filter.listener_ids =
		localFilter.listener_ids.length > 0 ? [...localFilter.listener_ids] : undefined
	itemsStore.pagination.currentPage = 1
	itemsStore.filter.offset = 0
	itemsStore.fetchItems(getClusterUrls())
}

const { drawerShow } = storeToRefs(configurationStore)

watch(
	localFilter,
	() => {
		applyFilter()
	},
	{ deep: true }
)

onUnmounted(() => {
	itemsStore.resetFilter()
	itemsStore.currentItem = null
	configurationStore.drawerShow = false
})
</script>

<template>
	<div class="w-full flex flex-col xl:flex-row gap-4 items-start">
		<AppLayoutPageContainer class="flex-1 min-w-0">
			<AppLayoutPageHeader title="Items">
				<template #actions>
					<AppFilterDropdown :active="hasActiveFilter" @reset="resetFilter">
						<AppInputAutocomplete
							v-model="localFilter.types"
							label="Type"
							placeholder="All Types"
							:options="itemsStore.selection.types"
							multiple
							:portal="false"
							clearable
						/>
						<AppInputAutocomplete
							v-model="localFilter.endpoint_ids"
							label="Endpoint"
							placeholder="All Endpoints"
							:options="itemsStore.selection.endpoints"
							sub-label-key="cluster_name"
							multiple
							remote
							:loading="itemsStore.loading.endpoints"
							:portal="false"
							clearable
							@search="onSearchEndpoints"
						/>
						<AppInputAutocomplete
							v-model="localFilter.listener_ids"
							label="Listener"
							placeholder="All Listeners"
							:options="itemsStore.selection.listeners"
							sub-label-key="cluster_name"
							multiple
							remote
							:loading="itemsStore.loading.listeners"
							:portal="false"
							clearable
							@search="onSearchListeners"
						/>
					</AppFilterDropdown>

					<UiButton @click="itemsStore.toggleItemsModalForm(true)">
						<UiIcon name="plus" size="16px" />
						<span>Create Item for Host</span>
					</UiButton>
				</template>
			</AppLayoutPageHeader>

			<items-table @toggle-drawer="drawerShow = true" />
		</AppLayoutPageContainer>

		<ConfigDrawer v-model="drawerShow" @close="itemsStore.currentItem = null">
			<template v-if="itemsStore.loading.form">
				<AppLoading container-height="32px" />
			</template>
			<Stats v-else :item="itemsStore.currentItem" />

			<template #footer>
				<div class="flex items-center gap-1.5 text-xs text-muted-foreground">
					<UiIcon name="refresh" size="14px" />
					<span>
						Last Update:
						<br />
						{{
							formatUtcDateTimeWithRelative(
								itemsStore.currentItem?.stats?.snapshot.last_attempt
							)
						}}
					</span>
				</div>
				<UiButton variant="outline" size="sm" @click="drawerShow = false"> Close </UiButton>
			</template>
		</ConfigDrawer>
	</div>
</template>

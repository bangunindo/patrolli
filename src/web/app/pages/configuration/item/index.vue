<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import appHelper from '@/app/helpers/app.helper'
import { formatUtcDateTimeWithRelative } from '@/app/helpers/date.helper'
import AppFilterDropdown from '@/components/app/AppFilterDropdown.vue'
import AppInputSelect from '@/components/app/input/AppInputSelect.vue'
import AppLoading from '@/components/app/loading/AppLoading.vue'
import ConfigDrawer from '@/components/app/navigation/ConfigDrawer.vue'
import Stats from '@/components/configuration/items/stats/Stats.vue'
import ItemsTable from '@/components/configuration/items/Table.vue'
import { useConfigurationStore } from '@/stores/configuration.store'
import { useItemsStore } from '@/stores/items.store'
import type { ItemType } from '@@/shared/types/item.type'
import { storeToRefs } from 'pinia'
import { onMounted, onUnmounted, reactive, watch } from 'vue'
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
	type: ItemType | '' | null
	endpoint_id: string | null
	listener_id: string | null
}>({
	type: null,
	endpoint_id: null,
	listener_id: null
})

onMounted(() => {
	const urls = getClusterUrls()
	itemsStore.fetchHosts(urls)
	itemsStore.fetchEndpoints(urls)
	itemsStore.fetchListeners(urls)

	localFilter.type = (itemsStore.filter.types?.[0] as any) || null
	localFilter.endpoint_id = itemsStore.filter.endpoint_ids?.[0] || null
	localFilter.listener_id = itemsStore.filter.listener_ids?.[0] || null
})

function applyFilter() {
	itemsStore.filter.types = localFilter.type ? [localFilter.type as ItemType] : undefined
	itemsStore.filter.endpoint_ids = localFilter.endpoint_id ? [localFilter.endpoint_id] : undefined
	itemsStore.filter.listener_ids = localFilter.listener_id ? [localFilter.listener_id] : undefined
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
	itemsStore.currentItem = null
	configurationStore.drawerShow = false
})
</script>

<template>
	<div class="w-full flex flex-col xl:flex-row gap-4 items-start">
		<AppLayoutPageContainer class="flex-1 min-w-0">
			<AppLayoutPageHeader title="Items">
				<template #actions>
					<AppFilterDropdown>
						<AppInputSelect
							v-model="localFilter.type"
							label="Type"
							placeholder="All Types"
							:options="itemsStore.selection.types"
							label-key="label"
							value-key="value"
							clearable
						/>
						<AppInputSelect
							v-model="localFilter.endpoint_id"
							label="Endpoint"
							placeholder="All Endpoints"
							:options="itemsStore.selection.endpoints"
							label-key="label"
							value-key="value"
							clearable
						/>
						<AppInputSelect
							v-model="localFilter.listener_id"
							label="Listener"
							placeholder="All Listeners"
							:options="itemsStore.selection.listeners"
							label-key="label"
							value-key="value"
							clearable
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

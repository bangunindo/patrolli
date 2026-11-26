<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import appHelper from '@/app/helpers/app.helper'
import AppFilterDropdown from '@/components/app/AppFilterDropdown.vue'
import AppInputAutocomplete from '@/components/app/input/AppInputAutocomplete.vue'
import EndpointTable from '@/components/configuration/endpoint/Table.vue'
import { useEndpointsStore } from '@/stores/endpoints.store'
import type { EndpointType } from '@@/shared/types/endpoint.type'
import { computed, onMounted, onUnmounted, reactive, watch } from 'vue'
import { useRoute } from 'vue-router'

definePageMeta({
	layout: 'configuration',
	requiresAuth: true
})

const route = useRoute()
const endpointsStore = useEndpointsStore()

const getClusterUrls = () => appHelper.getClusterUrls(route.query.cluster as string)

const localFilter = reactive<{
	types: EndpointType[]
	host_ids: string[]
}>({
	types: endpointsStore.filter.types ? [...endpointsStore.filter.types] : [],
	host_ids: endpointsStore.filter.host_ids ? [...endpointsStore.filter.host_ids] : []
})

onMounted(() => {
	endpointsStore.fetchHosts(getClusterUrls())
})

function onSearchHosts(query: string) {
	endpointsStore.fetchHosts(getClusterUrls(), query)
}

const hasActiveFilter = computed(
	() => localFilter.types.length > 0 || localFilter.host_ids.length > 0
)

function resetFilter() {
	localFilter.types = []
	localFilter.host_ids = []
}

function applyFilter() {
	endpointsStore.filter.types = localFilter.types.length > 0 ? [...localFilter.types] : undefined
	endpointsStore.filter.host_ids =
		localFilter.host_ids.length > 0 ? [...localFilter.host_ids] : undefined
	endpointsStore.pagination.currentPage = 1
	endpointsStore.filter.offset = 0
	endpointsStore.fetchEndpoints(getClusterUrls())
}

watch(
	localFilter,
	() => {
		applyFilter()
	},
	{ deep: true }
)

onUnmounted(() => {
	endpointsStore.resetFilter()
	endpointsStore.currentEndpoint = null
})
</script>

<template>
	<AppLayoutPageContainer>
		<AppLayoutPageHeader title="Endpoints">
			<template #actions>
				<AppFilterDropdown :active="hasActiveFilter" @reset="resetFilter">
					<AppInputAutocomplete
						v-model="localFilter.types"
						label="Type"
						placeholder="All Types"
						:options="endpointsStore.selection.types"
						multiple
						:portal="false"
						clearable
					/>
					<AppInputAutocomplete
						v-model="localFilter.host_ids"
						label="Host"
						placeholder="All Hosts"
						:options="endpointsStore.selection.hosts"
						sub-label-key="cluster_name"
						multiple
						remote
						:loading="endpointsStore.loading.hosts"
						:portal="false"
						clearable
						@search="onSearchHosts"
					/>
				</AppFilterDropdown>

				<UiButton @click="endpointsStore.toggleEndpointModalForm(true)">
					<UiIcon name="plus" size="16px" />
					<span>Create Endpoint</span>
				</UiButton>
			</template>
		</AppLayoutPageHeader>

		<endpoint-table />
	</AppLayoutPageContainer>
</template>

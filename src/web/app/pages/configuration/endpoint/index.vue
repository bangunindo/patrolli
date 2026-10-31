<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import AppFilterDropdown from '@/components/app/AppFilterDropdown.vue'
import AppInputSelect from '@/components/app/input/AppInputSelect.vue'
import EndpointTable from '@/components/configuration/endpoint/Table.vue'
import { useEndpointsStore } from '@/stores/endpoints.store'
import type { EndpointType } from '@@/shared/types/endpoint.type'
import { onMounted, reactive, watch } from 'vue'

definePageMeta({
	layout: 'configuration',
	requiresAuth: true
})

const endpointsStore = useEndpointsStore()

const localFilter = reactive<{
	type: EndpointType | null
	host_id: string | null
}>({
	type: null,
	host_id: null
})

onMounted(() => {
	endpointsStore.fetchHosts()
	localFilter.type = (endpointsStore.filter.types?.[0] as any) || null
	localFilter.host_id = endpointsStore.filter.host_ids?.[0] || null
})

function applyFilter() {
	endpointsStore.filter.types = localFilter.type ? [localFilter.type] : undefined
	endpointsStore.filter.host_ids = localFilter.host_id ? [localFilter.host_id] : undefined
	endpointsStore.filter.offset = 0
	endpointsStore.fetchEndpoints()
}

watch(
	localFilter,
	() => {
		applyFilter()
	},
	{ deep: true }
)
</script>

<template>
	<AppLayoutPageContainer>
		<AppLayoutPageHeader title="Endpoints">
			<template #actions>
				<AppFilterDropdown>
					<AppInputSelect
						v-model="localFilter.type"
						label="Type"
						placeholder="All Types"
						:options="endpointsStore.selection.types"
						label-key="label"
						value-key="value"
						clearable
					/>
					<AppInputSelect
						v-model="localFilter.host_id"
						label="Host"
						placeholder="All Hosts"
						:options="endpointsStore.selection.hosts"
						label-key="label"
						value-key="value"
						clearable
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

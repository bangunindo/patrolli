<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import AppFilterDropdown from '@/components/app/AppFilterDropdown.vue'
import AppInputSelect from '@/components/app/input/AppInputSelect.vue'
import SinkTable from '@/components/configuration/sinks/Table.vue'
import { useSinksStore } from '@/stores/sinks.store'
import { SinkConfigType } from '@@/shared/types/sink.type'
import { onMounted, reactive, watch } from 'vue'

definePageMeta({
	layout: 'configuration',
	requiresAuth: true
})

const sinksStore = useSinksStore()

const localFilter = reactive<{
	type: SinkConfigType | null
}>({
	type: null
})

onMounted(() => {
	localFilter.type = (sinksStore.filter.types?.[0] as any) || null
})

function applyFilter() {
	sinksStore.filter.types = localFilter.type ? [localFilter.type] : undefined
	sinksStore.filter.offset = 0
	sinksStore.fetchSinks()
}

watch(
	() => localFilter.type,
	(newVal, oldVal) => {
		if (newVal !== oldVal) {
			applyFilter()
		}
	}
)
</script>

<template>
	<AppLayoutPageContainer>
		<AppLayoutPageHeader title="Sinks">
			<template #actions>
				<AppFilterDropdown>
					<AppInputSelect
						v-model="localFilter.type"
						label="Type"
						placeholder="All Types"
						:options="sinksStore.selection.types"
						label-key="label"
						value-key="value"
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

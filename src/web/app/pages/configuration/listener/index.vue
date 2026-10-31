<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import AppFilterDropdown from '@/components/app/AppFilterDropdown.vue'
import AppInputSelect from '@/components/app/input/AppInputSelect.vue'
import ListenerTable from '@/components/configuration/listener/Table.vue'
import { useListenerStore } from '@/stores/listeners.store'
import type { ListenerType } from '@@/shared/types/listener.type'
import { onMounted, reactive, watch } from 'vue'

definePageMeta({
	layout: 'configuration',
	requiresAuth: true
})

const listenersStore = useListenerStore()

const localFilter = reactive<{ type: ListenerType | null }>({
	type: null
})

onMounted(() => {
	localFilter.type = (listenersStore.filter.types?.[0] as any) || null
})

function applyFilter() {
	listenersStore.filter.types = localFilter.type ? [localFilter.type] : undefined
	listenersStore.filter.offset = 0
	listenersStore.fetchListeners()
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
		<AppLayoutPageHeader title="Listener">
			<template #actions>
				<AppFilterDropdown>
					<AppInputSelect
						v-model="localFilter.type"
						label="Type"
						placeholder="All Types"
						:options="listenersStore.selection.types"
						label-key="label"
						value-key="value"
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

<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import AppFilterDropdown from '@/components/app/AppFilterDropdown.vue'
import AppInputSelect from '@/components/app/input/AppInputSelect.vue'
import ProcessorTable from '@/components/configuration/processor/Table.vue'
import { useProcessorStore } from '@/stores/processor.store'
import type { ProcessorType } from '@@/shared/types/processor.type'
import { onMounted, reactive, watch } from 'vue'

definePageMeta({
	layout: 'configuration',
	requiresAuth: true,
	middleware: [
		() => {
			return navigateTo('/configuration/host')
		}
	]
})

const processorStore = useProcessorStore()

const localFilter = reactive<{ type: ProcessorType | null }>({
	type: null
})

onMounted(() => {
	localFilter.type = (processorStore.filter.types?.[0] as any) || null
})

function applyFilter() {
	processorStore.filter.types = localFilter.type ? [localFilter.type] : undefined
	processorStore.filter.offset = 0
	processorStore.fetchProcessors()
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
		<AppLayoutPageHeader title="Processor">
			<template #actions>
				<AppFilterDropdown>
					<AppInputSelect
						v-model="localFilter.type"
						label="Type"
						placeholder="All Types"
						:options="processorStore.selection.modules"
						label-key="label"
						value-key="value"
						clearable
					/>
				</AppFilterDropdown>

				<UiButton @click="processorStore.toggleProcessorModalForm(true)">
					<UiIcon name="plus" size="16px" />
					<span>Create Processor</span>
				</UiButton>
			</template>
		</AppLayoutPageHeader>

		<processor-table />
	</AppLayoutPageContainer>
</template>

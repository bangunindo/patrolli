<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { onMounted, ref } from 'vue'

import AppTable from '@/components/app/AppTable.vue'
import ModalDetail from '@/components/configuration/processor/ModalDetail.vue'
import ModalProcessorForm from '@/components/configuration/processor/ModalForm.vue'

import ModalConfirmationDelete from '@/components/modal/ModalConfirmationDelete.vue'

import { useProcessorStore } from '@/stores/processor.store'
import { FormMode } from '@@/shared/types/app.type'
import type { IProcessor } from '@@/shared/types/processor.type'
import { watchDebounced } from '@vueuse/core'

const processorStore = useProcessorStore()

const showModalDelete = ref<boolean>(false)
const showModalDetail = ref<boolean>(false)

const headerTable = [
	{
		key: 'name',
		title: 'Name',
		sortable: true
	},
	{
		key: 'adm_status',
		title: 'Status',
		sortable: true
	},
	{
		key: 'module',
		title: 'Module'
	},
	{
		key: 'last_updated',
		title: 'Last Update'
	}
]

function onChangePage(payload) {
	if (processorStore.filter.offset != payload.offset) {
		processorStore.filter.limit = payload.limit
		processorStore.filter.offset = payload.offset
		processorStore.fetchProcessors()
	}
}

function toggleModalDelete(processor: IProcessor | null = null) {
	processorStore.currentProcessor = processor
	showModalDelete.value = !showModalDelete.value
}

function toggleModalDetail(processor: IProcessor | null = null) {
	processorStore.currentProcessor = processor
	showModalDetail.value = !showModalDetail.value
}

async function onDelete() {
	const clusterUrl = processorStore.currentProcessor?.cluster_url
	await processorStore.deleteProcessor(
		processorStore.currentProcessor?.processor_id ??
			(processorStore.currentProcessor as any)?.pipeline_id ??
			'',
		clusterUrl ? [clusterUrl] : undefined
	)
	showModalDelete.value = false
	processorStore.fetchProcessors()
}

onMounted(() => {
	processorStore.filter.offset = 0
	processorStore.fetchProcessors()
})

// search
watchDebounced(
	() => processorStore.filter.name,
	async () => {
		processorStore.filter.offset = 0
		processorStore.fetchProcessors()
	},
	{
		debounce: 500
	}
)
</script>

<template>
	<div class="w-full space-y-4">
		<div class="flex flex-col sm:flex-row items-center justify-between gap-3 w-full">
			<div class="relative w-full sm:w-80">
				<UiIcon
					name="magnify"
					size="18px"
					class="absolute left-3 top-1/2 -translate-y-1/2 text-muted-foreground pointer-events-none"
				/>
				<UiInput
					v-model="processorStore.filter.name"
					placeholder="Search by name..."
					class="pl-9 h-9 border-[#D8D9DB] dark:border-border rounded-md shadow-xs text-sm"
				/>
			</div>
		</div>

		<AppTable
			:columns="headerTable"
			:data="processorStore.processors"
			:loading="processorStore.loading.table"
			:pagination="processorStore.pagination"
			@update:page="onChangePage"
		>
			<template #col.name="prop">
				<div class="font-semibold">{{ prop.data.name }}</div>
				<span class="text-xs text-muted-foreground">{{ prop.data.cluster_name }}</span>
			</template>

			<template #col.adm_status="prop">
				<div class="flex flex-row flex-wrap gap-2 items-center">
					<div
						class="w-2.5 h-2.5 rounded-full"
						:class="
							Number(prop.data.adm_status) === 1 ? 'bg-emerald-500' : 'bg-rose-500'
						"
					></div>
					<div class="text-sm">
						{{ Number(prop.data.adm_status) === 1 ? 'Active' : 'Inactive' }}
					</div>
				</div>
			</template>

			<template #col.module="prop">
				<span class="capitalize text-sm">{{
					prop.data.config?.module ? prop.data.config.module.replace(/_/g, ' ') : '-'
				}}</span>
			</template>

			<template #col.last_updated="prop">
				<span class="text-sm">{{
					prop.data.last_updated ? prop.data.last_updated.formatDate() : '-'
				}}</span>
			</template>

			<template #col.action="prop">
				<div class="flex flex-row justify-end">
					<UiDropdownMenu>
						<UiDropdownMenuTrigger as-child>
							<UiButton
								variant="ghost"
								size="icon"
								class="h-8 w-8 text-muted-foreground hover:text-foreground"
							>
								<UiIcon name="dots-vertical" size="18px" />
							</UiButton>
						</UiDropdownMenuTrigger>

						<UiDropdownMenuContent align="end" class="w-44">
							<UiDropdownMenuItem
								class="cursor-pointer gap-2"
								@click="toggleModalDetail(prop.data)"
							>
								<UiIcon name="eye-outline" size="16px" />
								<span>View Detail</span>
							</UiDropdownMenuItem>

							<UiDropdownMenuItem
								class="cursor-pointer gap-2"
								@click="
									processorStore.toggleProcessorModalForm(
										true,
										FormMode.UPDATE,
										prop.data
									)
								"
							>
								<UiIcon name="pencil-outline" size="16px" />
								<span>Edit Processor</span>
							</UiDropdownMenuItem>

							<UiDropdownMenuSeparator />

							<UiDropdownMenuItem
								class="cursor-pointer gap-2 text-destructive focus:text-destructive focus:bg-destructive/10"
								@click="toggleModalDelete(prop.data)"
							>
								<UiIcon name="delete-outline" size="16px" />
								<span>Delete Processor</span>
							</UiDropdownMenuItem>
						</UiDropdownMenuContent>
					</UiDropdownMenu>
				</div>
			</template>
		</AppTable>
	</div>

	<ModalProcessorForm v-if="processorStore.processorModalShow" />

	<ModalDetail
		v-if="showModalDetail"
		:show-modal="showModalDetail"
		:processor="processorStore.currentProcessor ?? null"
		@dismiss="toggleModalDetail(null)"
	/>

	<ModalConfirmationDelete
		:show-modal="showModalDelete"
		@dismiss="showModalDelete = false"
		@confirm="onDelete"
	/>
</template>

<style scoped>
:deep(th > div.flex.flex-row.items-center > div) {
	width: unset;
}
</style>

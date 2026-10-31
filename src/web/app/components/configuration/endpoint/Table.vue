<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { useEndpointsStore } from '@/stores/endpoints.store'
import { FormMode } from '@@/shared/types/app.type'
import type { IEndpoint } from '@@/shared/types/endpoint.type'
import { watchDebounced } from '@vueuse/core'
import { storeToRefs } from 'pinia'
import { onMounted, ref } from 'vue'

import AppTable from '@/components/app/AppTable.vue'
import ModalDetail from '@/components/configuration/endpoint/ModalDetail.vue'
import ModalEndpointForm from '@/components/configuration/endpoint/ModalForm.vue'
import ModalConfirmationDelete from '@/components/modal/ModalConfirmationDelete.vue'

const endpointsStore = useEndpointsStore()

const { endpoints, endpointModalShow } = storeToRefs(endpointsStore)

const showModalDelete = ref<boolean>(false)
const showModalDetail = ref<boolean>(false)

const headerTable = [
	{
		key: 'name',
		title: 'Name',
		sortable: true
	},
	{
		key: 'host_name',
		title: 'Host'
	},
	{
		key: 'adm_status',
		title: 'Status',
		sortable: true
	},
	{
		key: 'type',
		title: 'Type'
	}
]

function onChangePage(payload) {
	if (endpointsStore.filter.offset != payload.offset) {
		endpointsStore.filter.limit = payload.limit
		endpointsStore.filter.offset = payload.offset
		endpointsStore.fetchEndpoints()
	}
}

function toggleModalDetail(endpoint: IEndpoint | null = null) {
	endpointsStore.currentEndpoint = endpoint
	showModalDetail.value = !showModalDetail.value
}

function toggleModalDelete(host: IEndpoint | null = null) {
	endpointsStore.currentEndpoint = host
	showModalDelete.value = !showModalDelete.value
}

async function onDelete() {
	const clusterUrl = endpointsStore.currentEndpoint?.cluster_url
	await endpointsStore.deleteEndpoint(
		endpointsStore.currentEndpoint?.endpoint_id ?? '',
		clusterUrl ? [clusterUrl] : undefined
	)
	showModalDelete.value = false
	endpointsStore.fetchEndpoints()
}

onMounted(() => {
	endpointsStore.filter.offset = 0
	endpointsStore.fetchEndpoints()
})

watchDebounced(
	() => endpointsStore.filter.name,
	async () => {
		endpointsStore.fetchEndpoints()
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
					v-model="endpointsStore.filter.name"
					placeholder="Search by name..."
					class="pl-9 h-9 border-[#D8D9DB] dark:border-border rounded-md shadow-xs text-sm"
				/>
			</div>
		</div>

		<AppTable
			v-bind="{
				columns: headerTable,
				data: endpoints,
				loading: endpointsStore.loading.table,
				pagination: endpointsStore.pagination
			}"
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
						:class="prop.data.adm_status === 1 ? 'bg-emerald-500' : 'bg-rose-500'"
					></div>
					<div class="text-sm">
						{{ prop.data.adm_status === 1 ? 'Active' : 'Inactive' }}
					</div>
				</div>
			</template>
			<template #col.type="prop">
				<UiBadge
					v-if="prop.data.config?.type"
					variant="secondary"
					class="font-normal text-xs uppercase tracking-wide"
				>
					{{ prop.data.config.type }}
				</UiBadge>
				<span v-else class="text-muted-foreground">-</span>
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
									endpointsStore.toggleEndpointModalForm(
										true,
										FormMode.UPDATE,
										prop.data
									)
								"
							>
								<UiIcon name="pencil-outline" size="16px" />
								<span>Edit Endpoint</span>
							</UiDropdownMenuItem>
							<UiDropdownMenuSeparator />
							<UiDropdownMenuItem
								class="cursor-pointer gap-2 text-destructive focus:text-destructive focus:bg-destructive/10"
								@click="toggleModalDelete(prop.data)"
							>
								<UiIcon name="delete-outline" size="16px" />
								<span>Delete Endpoint</span>
							</UiDropdownMenuItem>
						</UiDropdownMenuContent>
					</UiDropdownMenu>
				</div>
			</template>
		</AppTable>
	</div>

	<ModalEndpointForm v-if="endpointModalShow" />

	<ModalDetail
		v-if="showModalDetail"
		:show-modal="showModalDetail"
		:endpoint="endpointsStore.currentEndpoint ?? null"
		@dismiss="toggleModalDetail(null)"
	/>

	<ModalConfirmationDelete
		title="Delete Endpoint"
		:show-modal="showModalDelete"
		:message="`Are you sure you want to delete this endpoint?\n\nEndpoint: ${endpointsStore.currentEndpoint?.name}\n\nThis action cannot be undone.`"
		@dismiss="showModalDelete = false"
		@confirm="onDelete"
	/>
</template>

<style scoped>
:deep(th > div.flex.flex-row.items-center > div) {
	width: unset;
}
</style>

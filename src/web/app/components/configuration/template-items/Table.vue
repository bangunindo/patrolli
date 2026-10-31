<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { onMounted, ref } from 'vue'

import { useTemplateItemsStore } from '@/stores/template-items.store'

import appHelper from '@/app/helpers/app.helper'
import { formatSchedule } from '@/app/helpers/schedule.helper'
import AppTable from '@/components/app/AppTable.vue'
import ModalDetail from '@/components/configuration/template-items/ModalDetail.vue'
import ModalForm from '@/components/configuration/template-items/ModalForm.vue'
import ModalConfirmationDelete from '@/components/modal/ModalConfirmationDelete.vue'
import { FormMode } from '@@/shared/types/app.type'
import type { IItem } from '@@/shared/types/item.type'
import { watchDebounced } from '@vueuse/core'
import { useRoute } from 'vue-router'

const route = useRoute()
const templateItemsStore = useTemplateItemsStore()

const showModalDelete = ref<boolean>(false)
const showModalDetail = ref<boolean>(false)

const headerTable = [
	{
		key: 'resolved_name',
		title: 'Name',
		sortable: true
	},
	{
		key: 'resolved_schedule',
		title: 'Schedule'
	},
	{
		key: 'type',
		title: 'Type'
	},
	{
		key: 'resolved_adm_status',
		title: 'Status'
	},
	{
		key: 'resolved_outputs',
		title: 'Output'
	}
]

function onChangePage(payload: { offset: number; limit: number }) {
	if (templateItemsStore.filter.offset != payload.offset) {
		templateItemsStore.filter.limit = payload.limit
		templateItemsStore.filter.offset = payload.offset
		fetchItems()
	}
}

async function toggleModalDetail(item: IItem | null = null) {
	if (item?.item_id && !showModalDetail.value) {
		showModalDetail.value = true
		await templateItemsStore.fetchItemById(
			item.item_id,
			item.cluster_url ? [item.cluster_url] : undefined
		)
	} else {
		showModalDetail.value = false
		setTimeout(() => {
			templateItemsStore.currentItem = null
		}, 300)
	}
}

function toggleModalDelete(item: IItem | null = null) {
	templateItemsStore.currentItem = item
	showModalDelete.value = !showModalDelete.value
}

async function onDelete() {
	const clusterName = route.query.cluster as string
	const clusterUrl = appHelper.getClusterValue(clusterName)
	const urls = clusterUrl ? [clusterUrl] : undefined
	await templateItemsStore.deleteItem(templateItemsStore.currentItem?.item_id ?? '', urls)
	showModalDelete.value = false
	fetchItems()
}

async function fetchItems() {
	const clusterName = route.query.cluster as string
	const clusterUrl = appHelper.getClusterValue(clusterName)

	templateItemsStore.fetchItems([clusterUrl])
}

onMounted(() => {
	templateItemsStore.filter.offset = 0
	fetchItems()
})

// search
watchDebounced(
	() => templateItemsStore.filter.name,
	async () => {
		templateItemsStore.filter.offset = 0
		fetchItems()
	},
	{
		debounce: 500
	}
)
</script>

<template>
	<div class="space-y-4">
		<div class="relative w-full max-w-sm">
			<UiInput
				v-model="templateItemsStore.filter.name"
				placeholder="Search by name ..."
				class="pr-10"
			/>
			<UiIcon
				name="magnify"
				size="18px"
				class="absolute right-3 top-1/2 -translate-y-1/2 text-muted-foreground pointer-events-none"
			/>
		</div>

		<AppTable
			v-bind="{
				columns: headerTable,
				data: templateItemsStore.items,
				loading: templateItemsStore.loading.table,
				pagination: templateItemsStore.pagination
			}"
			@update:page="onChangePage"
		>
			<template #col.resolved_name="prop">
				<div class="whitespace-nowrap font-medium text-foreground">
					{{ prop.data.resolved_name || prop.data.name || '-' }}
				</div>
				<span class="text-xs text-muted-foreground">{{ prop.data.cluster_name }}</span>
			</template>

			<template #col.resolved_schedule="prop">
				<div class="whitespace-nowrap">
					<UiBadge
						v-if="prop.data.resolved_schedule"
						variant="outline"
						class="font-normal text-xs"
					>
						{{ formatSchedule(prop.data.resolved_schedule) }}
					</UiBadge>
					<span v-else class="text-muted-foreground">-</span>
				</div>
			</template>

			<template #col.type="prop">
				<UiBadge
					v-if="prop.data.type"
					variant="secondary"
					class="font-normal text-xs uppercase tracking-wide"
				>
					{{ String(prop.data.type).replaceAll('_', ' ') }}
				</UiBadge>
				<span v-else class="text-muted-foreground">-</span>
			</template>

			<template #col.resolved_adm_status="prop">
				<div class="flex flex-row whitespace-nowrap gap-2 items-center">
					<div
						class="w-2.5 h-2.5 rounded-full shrink-0"
						:class="
							Number(prop.data.resolved_adm_status) === 1
								? 'bg-emerald-500'
								: 'bg-rose-500'
						"
					/>
					<div class="text-sm">
						{{ Number(prop.data.resolved_adm_status) === 1 ? 'Active' : 'Inactive' }}
					</div>
				</div>
			</template>

			<template #col.resolved_outputs="prop">
				<span>{{
					prop.data.resolved_outputs ? prop.data.resolved_outputs?.length || 0 : '-'
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

						<UiDropdownMenuContent align="end" class="w-48">
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
									templateItemsStore.toggleItemsModalForm(
										true,
										FormMode.UPDATE,
										prop.data
									)
								"
							>
								<UiIcon name="pencil-outline" size="16px" />
								<span>Edit Template Item</span>
							</UiDropdownMenuItem>
							<UiDropdownMenuSeparator />
							<UiDropdownMenuItem
								class="cursor-pointer gap-2 text-destructive focus:text-destructive focus:bg-destructive/10"
								@click="toggleModalDelete(prop.data)"
							>
								<UiIcon name="delete-outline" size="16px" />
								<span>Delete Template Item</span>
							</UiDropdownMenuItem>
						</UiDropdownMenuContent>
					</UiDropdownMenu>
				</div>
			</template>
		</AppTable>
	</div>

	<ModalForm v-if="templateItemsStore.itemsModalShow" />

	<ModalDetail
		v-if="showModalDetail"
		:show-modal="showModalDetail"
		:item="templateItemsStore.currentItem ?? null"
		@dismiss="toggleModalDetail(null)"
	/>

	<ModalConfirmationDelete
		:show-modal="showModalDelete"
		:message="`Are you sure you want to delete this item?\nItem: ${templateItemsStore.currentItem?.name || templateItemsStore.currentItem?.resolved_name}\nThis action cannot be undone.`"
		@dismiss="showModalDelete = false"
		@confirm="onDelete"
	/>
</template>

<style scoped>
:deep(th > div.flex.flex-row.items-center > div) {
	width: unset;
	white-space: nowrap;
}
</style>

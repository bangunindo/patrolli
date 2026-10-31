<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { onMounted, ref } from 'vue'

import { useItemsStore } from '@/stores/items.store'

import AppTable from '@/components/app/AppTable.vue'
import ModalItemsForm from '@/components/configuration/items/ModalForm.vue'
import ModalItemsTemplateForm from '@/components/configuration/items/ModalFormTemplate.vue'

import appHelper from '@/app/helpers/app.helper'
import { formatSchedule } from '@/app/helpers/schedule.helper'
import ModalDetail from '@/components/configuration/items/ModalDetail.vue'
import ModalConfirmationDelete from '@/components/modal/ModalConfirmationDelete.vue'
import itemsConstant, { STATE_META_MAP } from '@@/shared/constants/items.constant'
import { FormMode } from '@@/shared/types/app.type'
import type { IItem } from '@@/shared/types/item.type'
import type { ITemplate } from '@@/shared/types/template.type'
import { watchDebounced } from '@vueuse/core'
import { useRoute, useRouter } from 'vue-router'

const emits = defineEmits(['toggleDrawer'])
const route = useRoute()
const router = useRouter()
const itemsStore = useItemsStore()

const showModalDelete = ref<boolean>(false)
const showModalDetail = ref<boolean>(false)

const getClusterUrls = (): string[] | undefined => {
	const clusterUrl = appHelper.getClusterValue(route.query.cluster as string)
	return clusterUrl ? [clusterUrl] : undefined
}

const headerTable = [
	{
		key: 'name',
		title: 'Name',
		sortable: true
	},
	{
		key: 'type',
		title: 'Type'
	},
	{
		key: 'host_name',
		title: 'Host Name'
	},
	{
		key: 'template',
		title: 'Template'
	},
	{
		key: 'resolved_adm_status',
		title: 'Status'
	},
	{
		key: 'health',
		title: 'Collection Health'
	},
	{
		key: 'output',
		title: 'Output'
	}
]

function onChangePage(payload: { offset: number; limit: number }) {
	if (itemsStore.filter.offset != payload.offset) {
		itemsStore.filter.limit = payload.limit
		itemsStore.filter.offset = payload.offset
		itemsStore.fetchItems()
	}
}

async function toggleModalDetail(item: IItem | null = null) {
	if (item?.item_id && !showModalDetail.value) {
		showModalDetail.value = true
		await itemsStore.fetchItemById(
			item.item_id,
			item.cluster_url ? [item.cluster_url] : getClusterUrls()
		)
	} else {
		showModalDetail.value = false
		setTimeout(() => {
			itemsStore.currentItem = null
		}, 300)
	}
}

async function toggleDrawer(item: IItem) {
	await itemsStore.fetchItemById(
		item.item_id,
		item.cluster_url ? [item.cluster_url] : getClusterUrls()
	)
	emits('toggleDrawer', item)
}

function toggleModalDelete(item: IItem | null = null) {
	itemsStore.currentItem = item
	showModalDelete.value = !showModalDelete.value
}

async function onDelete() {
	const current = itemsStore.currentItem
	const clusterUrl =
		current?.cluster_url || appHelper.getClusterValue(route.query.cluster as string)
	const urls = clusterUrl ? [clusterUrl] : undefined
	await itemsStore.deleteItem(current?.item_id ?? '', urls)
	showModalDelete.value = false
	itemsStore.fetchItems()
}

function navigateToItems(template: ITemplate) {
	if (template.template_id == null) return
	router.push(
		`/configuration/template/${template.template_id}/items?cluster=${template.cluster_name}`
	)
}

function getIconFromState(state?: string) {
	if (!state) return 'information-outline'
	const icon = itemsConstant.icon.state[state]
	return icon ?? 'information-outline'
}

onMounted(() => {
	itemsStore.filter.offset = 0
	itemsStore.fetchItems()
	itemsStore.fetchEndpoints()
	itemsStore.fetchListeners()
})

// search
watchDebounced(
	() => itemsStore.filter.name,
	async () => {
		itemsStore.filter.offset = 0
		itemsStore.fetchItems()
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
					v-model="itemsStore.filter.name"
					placeholder="Search by name..."
					class="pl-9 h-9 border-[#D8D9DB] dark:border-border rounded-md shadow-xs text-sm"
				/>
			</div>
		</div>

		<AppTable
			v-bind="{
				columns: headerTable,
				data: itemsStore.items,
				loading: itemsStore.loading.table,
				pagination: itemsStore.pagination
			}"
			@update:page="onChangePage"
		>
			<template #col.name="prop">
				<section class="flex items-center gap-2">
					<UiIcon
						:name="getIconFromState(prop.data?.stats?.snapshot?.state)"
						size="20px"
						class="text-muted-foreground shrink-0"
					/>
					<div>
						<div class="whitespace-nowrap font-medium text-foreground">
							{{ prop.data.resolved_name || prop.data.name || '-' }}
						</div>
						<div class="text-xs text-muted-foreground">
							{{ formatSchedule(prop.data.resolved_schedule) }}
						</div>
					</div>
				</section>
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

			<template #col.host_name="prop">
				<div class="whitespace-nowrap font-medium text-foreground">
					{{ prop.data.host_name ?? '-' }}
				</div>
				<span class="text-xs text-muted-foreground">{{ prop.data.cluster_name }}</span>
			</template>

			<template #col.template="prop">
				<UiButton
					v-if="prop.data?.template_name"
					variant="link"
					size="sm"
					class="h-auto p-0 text-primary gap-1 font-normal"
					@click="navigateToItems(prop.data)"
				>
					<UiIcon name="format-list-bulleted" size="14px" />
					<span class="whitespace-nowrap">{{ prop.data?.template_name }}</span>
				</UiButton>
				<span v-else class="text-muted-foreground">-</span>
			</template>

			<template #col.resolved_adm_status="prop">
				<div class="flex flex-row whitespace-nowrap items-center gap-2">
					<div
						class="w-2.5 h-2.5 rounded-full shrink-0"
						:class="
							Number(prop.data.resolved_adm_status) === 1
								? 'bg-emerald-500'
								: 'bg-rose-500'
						"
					/>
					<span class="text-sm">
						{{ Number(prop.data.resolved_adm_status) === 1 ? 'Active' : 'Inactive' }}
					</span>
				</div>
			</template>

			<template #col.health="prop">
				<section v-if="prop.data?.stats?.snapshot">
					<div class="flex flex-row whitespace-nowrap items-center gap-1.5">
						<div
							class="w-2.5 h-2.5 rounded-full shrink-0"
							:class="
								STATE_META_MAP[prop.data.stats.snapshot.state]?.dotClass ??
								'bg-slate-400'
							"
						/>
						<span class="font-medium text-xs uppercase">
							{{ prop.data.stats.snapshot.state }}
						</span>
					</div>
					<div class="text-xs text-muted-foreground mt-0.5">
						Last: {{ prop.data.stats.snapshot.last_attempt ?? '-' }}
					</div>
					<div class="text-xs font-medium text-foreground">
						Checks: {{ prop.data.stats.snapshot.total_checks ?? 0 }} rows
					</div>
				</section>
				<span v-else class="text-muted-foreground">-</span>
			</template>

			<template #col.output="prop">
				<section v-if="prop.data?.stats?.snapshot" class="text-xs space-y-0.5">
					<div>Records: {{ prop.data.stats.snapshot.total_records ?? 0 }} rows</div>
					<div>Dropped: {{ prop.data.stats.snapshot.total_dropped ?? 0 }} rows</div>
					<div>Fails: {{ prop.data.stats.snapshot.total_failures ?? 0 }} rows</div>
				</section>
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
								@click="toggleDrawer(prop.data)"
							>
								<UiIcon name="chart-line" size="16px" />
								<span>View Trends</span>
							</UiDropdownMenuItem>
							<UiDropdownMenuItem
								class="cursor-pointer gap-2"
								@click="
									itemsStore.toggleItemsModalForm(
										true,
										FormMode.UPDATE,
										prop.data
									)
								"
							>
								<UiIcon name="pencil-outline" size="16px" />
								<span>Edit Item</span>
							</UiDropdownMenuItem>
							<UiDropdownMenuSeparator />
							<UiDropdownMenuItem
								class="cursor-pointer gap-2 text-destructive focus:text-destructive focus:bg-destructive/10"
								@click="toggleModalDelete(prop.data)"
							>
								<UiIcon name="delete-outline" size="16px" />
								<span>Delete Item</span>
							</UiDropdownMenuItem>
						</UiDropdownMenuContent>
					</UiDropdownMenu>
				</div>
			</template>
		</AppTable>
	</div>

	<ModalItemsForm v-if="itemsStore.itemsModalShow" />
	<ModalItemsTemplateForm v-if="itemsStore.itemsTemplateModalShow" />

	<ModalDetail
		v-if="showModalDetail"
		:show-modal="showModalDetail"
		:item="itemsStore.currentItem ?? null"
		@dismiss="toggleModalDetail(null)"
	/>

	<ModalConfirmationDelete
		:show-modal="showModalDelete"
		:message="`Are you sure you want to delete this item?\nItem: ${itemsStore.currentItem?.name || itemsStore.currentItem?.resolved_name}\nThis action cannot be undone.`"
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

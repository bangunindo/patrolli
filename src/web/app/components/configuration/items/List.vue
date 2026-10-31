<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import AppInputEndpoint from '@/components/app/input/AppInputEndpoint.vue'
import AppInputHost from '@/components/app/input/AppInputHost.vue'
import AppInputListener from '@/components/app/input/AppInputListener.vue'
import AppInputSelect from '@/components/app/input/AppInputSelect.vue'
import AppInputText from '@/components/app/input/AppInputText.vue'
import FieldGroup from '@/components/app/input/FieldGroup.vue'
import ScrollContainer from '@/components/app/ScrollContainer.vue'
import ModalDetail from '@/components/configuration/items/ModalDetail.vue'
import ModalItemsForm from '@/components/configuration/items/ModalForm.vue'
import ModalConfirmationDelete from '@/components/modal/ModalConfirmationDelete.vue'
import { useItemsStore } from '@/stores/items.store'
import { usePipelineBuildStore } from '@/stores/pipeline.builder.store'
import { FormMode } from '@@/shared/types/app.type'
import type { INode } from '@@/shared/types/canvas.type'
import type { IItem } from '@@/shared/types/item.type'
import { storeToRefs } from 'pinia'
import { onMounted, ref } from 'vue'

const props = withDefaults(
	defineProps<{
		processorForm?: any
		formInputItems?: string[]
		node: INode
	}>(),
	{}
)

const store = usePipelineBuildStore()
const itemsStore = useItemsStore()

const { itemPagination, items, itemFilter, itemLoading } = storeToRefs(store)

const showModalDelete = ref<boolean>(false)
const showModalDetail = ref<boolean>(false)

const selectedItem = ref<any[]>([])

const loadMore = async (payload) => {
	store.itemPagination.currentPage = payload.page
	store.itemFilter.offset = payload.offset

	await store.fetchItems()
}

const onSearchItem = async () => {
	store.items = []
	store.itemPagination.currentPage = 1
	store.itemFilter.offset = 0

	await store.fetchItems(true)
	store.itemPagination.reachEnd = false
}

const onAfterSubmit = async () => {
	store.fetchItems(true)
	store.updateCanvas()
}

async function toggleModalDetail(item: IItem | null = null) {
	if (item?.item_id && !showModalDetail.value) {
		showModalDetail.value = true
		await itemsStore.fetchItemById(
			item.item_id,
			item.cluster_url ? [item.cluster_url] : undefined
		)
	} else {
		showModalDetail.value = false
		setTimeout(() => {
			itemsStore.currentItem = null
		}, 300)
	}
}

function toggleModalDelete(item: IItem | null = null) {
	itemsStore.currentItem = item
	showModalDelete.value = !showModalDelete.value
}

async function onDelete() {
	const current = itemsStore.currentItem
	const urls = current?.cluster_url ? [current.cluster_url] : undefined
	await itemsStore.deleteItem(current?.item_id ?? '', urls)
	showModalDelete.value = false
	onAfterSubmit()
}

onMounted(() => {
	store.itemPagination.reachEnd = false
	store.itemPagination.currentPage = 1

	store.itemFilter.offset = 0
	store.itemFilter.exclusivity = ['host', 'template_on_host']

	store.items = []

	store.fetchItems()
})

store.$onAction((context) => {
	if (context.name == 'fetchItems') {
		context.after(() => {
			const inputItems = props.formInputItems ?? []

			if (inputItems.length > 0) {
				selectedItem.value = store.items.filter((item) => {
					return item.item_id.includeTexts(inputItems)
				})
			}
		})
	}
})
</script>

<template>
	<section class="py-3">
		<form @submit.prevent="onSearchItem">
			<div class="flex flex-wrap gap-3 items-end">
				<app-input-text
					v-model="itemFilter.name"
					class="min-w-[250px] flex-1 flex-grow"
					placeholder="Search Items"
					variant="outline"
					pre-icon="magnify"
				/>

				<app-input-select
					v-model="itemFilter.types"
					:options="itemsStore.selection.types"
					class="min-w-[250px] flex-1 flex-grow"
					placeholder="Select type"
					variant="outline"
					pre-icon="magnify"
				/>

				<app-input-host v-model="itemFilter.host_ids" />

				<app-input-endpoint v-model="itemFilter.endpoint_ids" />

				<app-input-listener v-model="itemFilter.listener_ids" />
			</div>

			<div class="flex-1 flex justify-end mt-3">
				<UiButton type="submit" class="w-full justify-center"> Filter </UiButton>
			</div>
		</form>
	</section>

	<scroll-container
		:is-fetching="itemLoading.table"
		:max-height="'calc(100vh - 575px)'"
		:pagination="itemPagination"
		@infinite="loadMore"
	>
		<section style="min-height: 325px" class="">
			<field-group label="Items">
				<template v-for="(item, i) in items" :key="i">
					<div class="flex justify-between items-center py-2">
						<span class="text-sm font-medium">{{ item.resolved_name }}</span>
						<UiBadge variant="outline">{{ item.type.toUpperCase() }}</UiBadge>
						<div class="flex flex-row justify-center gap-1.5">
							<UiButton
								variant="outline"
								size="icon"
								class="h-8 w-8 text-sky-500 hover:text-sky-600"
								@click="toggleModalDetail(item)"
							>
								<UiIcon name="eye" class="w-4 h-4" />
							</UiButton>

							<UiButton
								variant="outline"
								size="icon"
								class="h-8 w-8 text-amber-500 hover:text-amber-600"
								@click="
									itemsStore.toggleItemsModalForm(true, FormMode.UPDATE, item)
								"
							>
								<UiIcon name="pencil-outline" class="w-4 h-4" />
							</UiButton>

							<UiButton
								variant="outline"
								size="icon"
								class="h-8 w-8 text-rose-500 hover:text-rose-600"
								@click="toggleModalDelete(item)"
							>
								<UiIcon name="delete-outline" class="w-4 h-4" />
							</UiButton>
						</div>
					</div>
					<hr v-if="i < items.length - 1" class="border-t border-border my-1" />
				</template>

				<template v-if="items.length == 0">
					<div class="text-xs text-muted-foreground py-2">No data Available</div>
				</template>
			</field-group>
		</section>
	</scroll-container>

	<section class="pt-6">
		<div class="flex flex-col gap-2">
			<UiButton
				variant="outline"
				class="text-primary w-full"
				@click="itemsStore.toggleItemsModalForm(true)"
			>
				<UiIcon name="plus" class="w-4 h-4 mr-2" />
				Create Item for Host
			</UiButton>
		</div>
	</section>

	<modal-items-form v-if="itemsStore.itemsModalShow" @after:submit="onAfterSubmit" />

	<modal-detail
		v-if="showModalDetail"
		:show-modal="showModalDetail"
		:item="itemsStore.currentItem ?? null"
		@dismiss="toggleModalDetail(null)"
	/>

	<modal-confirmation-delete
		:show-modal="showModalDelete"
		:message="`Are you sure you want to delete this item?\nItem: ${itemsStore.currentItem?.name || itemsStore.currentItem?.resolved_name}\nThis action cannot be undone.`"
		@dismiss="showModalDelete = false"
		@confirm="onDelete"
	/>
</template>

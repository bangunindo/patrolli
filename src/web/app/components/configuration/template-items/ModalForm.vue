<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { useItemsStore } from '@/stores/items.store'
import { useTemplateItemsStore } from '@/stores/template-items.store'
import { storeToRefs } from 'pinia'
import { watch } from 'vue'

import ItemFormTemplate from '@/components/configuration/items/FormTemplate.vue'

const templateItemsStore = useTemplateItemsStore()
const itemsStore = useItemsStore()

const { itemsModalShow, itemsFormMode, currentItem } = storeToRefs(templateItemsStore)

function onAfterSubmit() {
	templateItemsStore.fetchItems()
	templateItemsStore.toggleItemsModalForm(false)
}

watch(
	itemsModalShow,
	(val) => {
		if (val) {
			itemsStore.toggleItemsTemplateModalForm(true, itemsFormMode.value, currentItem.value)
		} else {
			itemsStore.toggleItemsTemplateModalForm(false)
		}
	},
	{ immediate: true }
)

watch(
	() => itemsStore.itemsTemplateModalShow,
	(val) => {
		if (!val && itemsModalShow.value) {
			templateItemsStore.toggleItemsModalForm(false)
		}
	}
)
</script>

<template>
	<UiDialog v-model:open="itemsStore.itemsTemplateModalShow">
		<UiDialogContent class="max-w-2xl max-h-[85vh] flex flex-col p-6">
			<ItemFormTemplate
				v-if="itemsStore.itemsTemplateModalShow"
				@after:submit="onAfterSubmit"
			/>
		</UiDialogContent>
	</UiDialog>
</template>

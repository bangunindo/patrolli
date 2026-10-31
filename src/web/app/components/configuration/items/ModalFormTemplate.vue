<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { useItemsStore } from '@/stores/items.store'
import { storeToRefs } from 'pinia'

import ItemFormTemplate from '@/components/configuration/items/FormTemplate.vue'

const itemsStore = useItemsStore()

const { itemsTemplateModalShow } = storeToRefs(itemsStore)
</script>

<template>
	<UiDialog v-model:open="itemsTemplateModalShow">
		<UiDialogContent class="max-w-2xl max-h-[85vh] flex flex-col p-6">
			<ItemFormTemplate />
		</UiDialogContent>
	</UiDialog>
</template>

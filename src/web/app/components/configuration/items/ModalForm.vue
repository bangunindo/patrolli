<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { useItemsStore } from '@/stores/items.store'
import { storeToRefs } from 'pinia'

import ItemForm from '@/components/configuration/items/Form.vue'

const emits = defineEmits(['after:submit'])
const itemsStore = useItemsStore()

const { itemsModalShow } = storeToRefs(itemsStore)
</script>

<template>
	<UiDialog v-model:open="itemsModalShow">
		<UiDialogContent class="max-w-2xl max-h-[85vh] flex flex-col p-6">
			<ItemForm @after:submit="emits('after:submit')" />
		</UiDialogContent>
	</UiDialog>
</template>

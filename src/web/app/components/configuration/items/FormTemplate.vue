<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import ItemForm from '@/components/configuration/items/Form.vue'

const emits = defineEmits(['after:submit'])
const props = withDefaults(
	defineProps<{
		showTitle?: boolean
		showCancel?: boolean
		footerFull?: boolean
	}>(),
	{
		showTitle: true,
		showCancel: true,
		footerFull: false
	}
)
</script>

<template>
	<ItemForm :is-template-context="true" v-bind="props" @after:submit="emits('after:submit')" />
</template>

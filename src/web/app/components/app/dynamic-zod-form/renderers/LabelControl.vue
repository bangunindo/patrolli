<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import type { FieldDescriptor } from '@@/shared/types/dynamic-form.type'
import AppInputLabel from '@/components/app/input/Label.vue'
import CustomSlotRenderer from './CustomSlotRenderer.vue'
import { useDynamicField } from '../useDynamicField'

const props = defineProps<{
	descriptor: FieldDescriptor
	path: string
}>()

const { value, setValue, error, label, required, disabled, customSlot, slotProps } =
	useDynamicField(props)

const onInput = (val: string) => {
	setValue(val)
}
</script>

<template>
	<div :data-invalid="!!error" :class="{ 'has-error': !!error }">
		<CustomSlotRenderer v-if="customSlot" :render-fn="customSlot" :slot-props="slotProps" />

		<AppInputLabel
			v-else
			:model-value="typeof value === 'string' ? value : JSON.stringify(value || {})"
			:label="label || 'Labels'"
			:required="required"
			:error="error"
			:disabled="disabled"
			@update:model-value="onInput"
		/>
	</div>
</template>

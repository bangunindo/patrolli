<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import type { FieldDescriptor } from '@@/shared/types/dynamic-form.type'
import AppInputText from '@/components/app/input/AppInputText.vue'
import CustomSlotRenderer from './CustomSlotRenderer.vue'
import { useDynamicField } from '../useDynamicField'

const props = defineProps<{
	descriptor: FieldDescriptor
	path: string
}>()

const { value, setValue, error, hint, label, required, disabled, customSlot, slotProps } =
	useDynamicField(props)

const onInput = (val: any) => {
	if (val === '' || val === null || val === undefined) {
		const emptyVal = hint.value.emptyPolicy === 'null' ? null : undefined
		setValue(emptyVal)
		return
	}
	const num = Number(val)
	setValue(isNaN(num) ? val : num)
}
</script>

<template>
	<div :data-invalid="!!error" :class="{ 'has-error': !!error }">
		<CustomSlotRenderer v-if="customSlot" :render-fn="customSlot" :slot-props="slotProps" />

		<AppInputText
			v-else
			type="number"
			:model-value="value ?? ''"
			:label="label"
			:placeholder="hint.placeholder || descriptor.placeholder || `Enter ${label}`"
			:required="required"
			:error="error"
			:disabled="disabled"
			@update:model-value="onInput"
		/>
	</div>
</template>

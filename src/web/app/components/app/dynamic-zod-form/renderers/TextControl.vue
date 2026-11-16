<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed } from 'vue'
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

const displayValue = computed(() => {
	const d = value.value
	if (d === null || d === undefined) return ''
	if (typeof d === 'object') {
		const obj = d as Record<string, any>
		return obj.String ?? obj.SmolStr ?? ''
	}
	return String(d)
})

const inputType = computed(() => {
	const h = hint.value as Record<string, any>
	if (h.widget === 'textarea' || h.type === 'textarea') return 'textarea'
	if (h.widget === 'date' || h.type === 'date' || h.format === 'date') return 'date'
	return h.type || 'text'
})

const onInput = (val: string | number) => {
	let finalVal: any = val
	if (val === '' && hint.value.emptyPolicy === 'null') {
		finalVal = null
	}
	setValue(finalVal)
}
</script>

<template>
	<div :data-invalid="!!error" :class="{ 'has-error': !!error, contents: !!customSlot }">
		<CustomSlotRenderer v-if="customSlot" :render-fn="customSlot" :slot-props="slotProps" />

		<AppInputText
			v-else
			:type="inputType"
			:model-value="displayValue"
			:label="label"
			:placeholder="hint.placeholder || descriptor.placeholder || `Enter ${label}`"
			:required="required"
			:error="error"
			:disabled="disabled"
			:uuid="
				hint.format === 'uuid' ||
				descriptor.format === 'uuid' ||
				hint.rules?.includes('uuid') ||
				descriptor.rules?.includes('uuid')
			"
			:rules="hint.rules || descriptor.rules"
			@update:model-value="onInput"
		/>
	</div>
</template>

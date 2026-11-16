<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed, watch } from 'vue'
import type { FieldDescriptor } from '@@/shared/types/dynamic-form.type'
import AppInputSelect from '@/components/app/input/AppInputSelect.vue'
import CustomSlotRenderer from './CustomSlotRenderer.vue'
import { useDynamicField } from '../useDynamicField'

const props = defineProps<{
	descriptor: FieldDescriptor
	path: string
}>()

const { value, setValue, error, hint, label, required, disabled, customSlot, slotProps } =
	useDynamicField(props)

const selectOptions = computed(() => {
	if (Array.isArray(hint.value.options)) {
		return hint.value.options
	}
	return props.descriptor.options || []
})

const isEmptyForDefault = computed(() => {
	if (value.value === undefined || value.value === '') return true
	if (value.value === null && hint.value.emptyPolicy !== 'null') return true
	return false
})

watch(
	[
		() => hint.value.defaultValue,
		() => props.descriptor.defaultValue,
		() => selectOptions.value,
		() => required.value,
		() => value.value
	],
	([hintDefault, descDefault, opts, isReq, currentVal]) => {
		const defaultVal = hintDefault !== undefined ? hintDefault : descDefault
		if (defaultVal !== undefined && currentVal !== defaultVal && isEmptyForDefault.value) {
			setValue(defaultVal)
		} else if (opts.length === 1 && isReq && isEmptyForDefault.value) {
			setValue(opts[0].value)
		}
	},
	{ immediate: true }
)

const onSelect = (val: any) => {
	let finalVal = val
	if ((val === '' || val === undefined) && hint.value.emptyPolicy === 'null') {
		finalVal = null
	}
	setValue(finalVal)
}
</script>

<template>
	<div :data-invalid="!!error" :class="{ 'has-error': !!error }">
		<CustomSlotRenderer v-if="customSlot" :render-fn="customSlot" :slot-props="slotProps" />

		<AppInputSelect
			v-else
			:model-value="value"
			:label="label"
			:placeholder="hint.placeholder || descriptor.placeholder || `Select ${label}`"
			:required="required"
			:error="error"
			:disabled="disabled"
			:options="selectOptions"
			@update:model-value="onSelect"
		/>
	</div>
</template>

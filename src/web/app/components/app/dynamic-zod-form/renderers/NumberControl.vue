<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed, inject } from 'vue'
import { rendererProps, useJsonFormsControl } from '@jsonforms/vue'
import type { ControlElement } from '@jsonforms/core'
import AppInputText from '@/components/app/input/AppInputText.vue'
import CustomSlotRenderer from './CustomSlotRenderer.vue'

const props = defineProps({
	...rendererProps<ControlElement>()
})

const { control, handleChange } = useJsonFormsControl(props)
const slots = inject<Record<string, any>>('dynamicZodFormSlots', {})

const leafName = computed(() => {
	const p = control.value.path || ''
	const parts = p.split('.')
	return parts[parts.length - 1] || p
})

const customSlot = computed(() => {
	const pathKey = control.value.path
	return slots[`field:${pathKey}`] || slots[`field:${leafName.value}`] || null
})

const onInput = (val: any) => {
	if (val === '' || val === null || val === undefined) {
		const emptyVal = control.value.uischema?.options?.emptyPolicy === 'null' ? null : undefined
		handleChange(control.value.path, emptyVal)
		return
	}
	const num = Number(val)
	handleChange(control.value.path, isNaN(num) ? val : num)
}

const slotProps = computed(() => ({
	value: control.value.data,
	modelValue: control.value.data,
	'model-value': control.value.data,
	error: control.value.errors,
	updateModelValue: onInput,
	'update-model-value': onInput,
	handleChange: onInput,
	'handle-change': onInput
}))
</script>

<template>
	<div
		v-if="control.visible"
		:data-invalid="!!control.errors"
		:class="{ 'has-error': !!control.errors }"
	>
		<!-- Custom Slot Fallback (e.g. #field:adm_status, #field:flags) -->
		<CustomSlotRenderer v-if="customSlot" :render-fn="customSlot" :slot-props="slotProps" />

		<AppInputText
			v-else
			type="number"
			:model-value="control.data ?? ''"
			:label="control.label"
			:placeholder="control.uischema?.options?.placeholder || `Enter ${control.label}`"
			:required="control.required"
			:error="control.errors"
			:disabled="!control.enabled"
			@update:model-value="onInput"
		/>
	</div>
</template>

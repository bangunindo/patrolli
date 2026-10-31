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

const displayValue = computed(() => {
	const d = control.value.data
	if (d === null || d === undefined) return ''
	if (typeof d === 'object') {
		return d.String ?? d.SmolStr ?? ''
	}
	return String(d)
})

const inputType = computed(() => {
	const opts = control.value.uischema?.options || {}
	if (opts.widget === 'textarea' || opts.type === 'textarea') return 'textarea'
	if (opts.widget === 'date' || opts.type === 'date' || opts.format === 'date') return 'date'
	return opts.type || 'text'
})

const onInput = (val: string | number) => {
	let finalVal = val
	if (val === '' && control.value.uischema?.options?.emptyPolicy === 'null') {
		finalVal = null as any
	}
	handleChange(control.value.path, finalVal)
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
		:class="{ 'has-error': !!control.errors, contents: !!customSlot }"
	>
		<!-- Custom Slot Fallback (e.g. #field:host_id) -->
		<CustomSlotRenderer v-if="customSlot" :render-fn="customSlot" :slot-props="slotProps" />

		<!-- Default BTJ Input Text -->
		<AppInputText
			v-else
			:type="inputType"
			:model-value="displayValue"
			:label="control.label"
			:placeholder="control.uischema?.options?.placeholder || `Enter ${control.label}`"
			:required="control.required"
			:error="control.errors"
			:disabled="!control.enabled"
			:uuid="
				control.uischema?.options?.format === 'uuid' ||
				control.uischema?.options?.rules?.includes('uuid')
			"
			:rules="control.uischema?.options?.rules"
			@update:model-value="onInput"
		/>
	</div>
</template>

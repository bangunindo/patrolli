<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed, inject } from 'vue'
import { rendererProps, useJsonFormsControl } from '@jsonforms/vue'
import type { ControlElement } from '@jsonforms/core'
import AppInputError from '@/components/app/input/AppInputError.vue'
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

const isChecked = computed(() => {
	if (typeof control.value.data === 'number') {
		return control.value.data === 1
	}
	return Boolean(control.value.data)
})

const onToggle = (val: boolean) => {
	const isNumber =
		control.value.schema.type === 'number' || control.value.schema.type === 'integer'
	handleChange(control.value.path, isNumber ? (val ? 1 : 0) : val)
}

const onCustomInput = (val: any) => {
	handleChange(control.value.path, val)
}

const slotProps = computed(() => ({
	value: control.value.data,
	modelValue: control.value.data,
	'model-value': control.value.data,
	error: control.value.errors,
	updateModelValue: onCustomInput,
	'update-model-value': onCustomInput,
	handleChange: onCustomInput,
	'handle-change': onCustomInput
}))
</script>

<template>
	<div
		v-if="control.visible"
		:data-invalid="!!control.errors"
		:class="{ 'has-error': !!control.errors }"
	>
		<!-- Custom Slot Fallback (e.g. #field:adm_status) -->
		<CustomSlotRenderer v-if="customSlot" :render-fn="customSlot" :slot-props="slotProps" />

		<!-- Default Switch Control -->
		<div v-else class="flex flex-col gap-1 p-1">
			<label class="font-semibold block text-sm text-foreground">
				{{ control.label }}
				<span v-if="control.required" class="text-destructive">*</span>
			</label>
			<div class="flex items-center gap-2 pt-1">
				<UiSwitch
					:checked="isChecked"
					:disabled="!control.enabled"
					@update:checked="onToggle"
				/>
				<span class="text-sm font-normal text-muted-foreground">
					{{ isChecked ? 'Active' : 'Inactive' }}
				</span>
			</div>
			<AppInputError :error="control.errors" />
		</div>
	</div>
</template>

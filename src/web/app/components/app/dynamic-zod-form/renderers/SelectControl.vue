<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed, watchEffect } from 'vue'
import { rendererProps, useJsonFormsControl } from '@jsonforms/vue'
import type { ControlElement } from '@jsonforms/core'
import AppInputSelect from '@/components/app/input/AppInputSelect.vue'

const props = defineProps({
	...rendererProps<ControlElement>()
})

const { control, handleChange } = useJsonFormsControl(props)

const selectOptions = computed(() => {
	if (control.value.uischema?.options?.options) {
		return control.value.uischema.options.options
	}
	if (control.value.schema.enum) {
		return control.value.schema.enum.map((val: any) => ({
			label: String(val),
			value: val
		}))
	}
	if (control.value.schema.oneOf) {
		return (control.value.schema.oneOf as any[]).map((sub: any) => ({
			label: sub.title || String(sub.const || sub.default || sub.enum?.[0] || ''),
			value: sub.const ?? sub.default ?? sub.enum?.[0] ?? ''
		}))
	}
	if (control.value.schema.anyOf) {
		const enumSub = (control.value.schema.anyOf as any[]).find(
			(sub: any) => sub && sub.type !== 'null' && (sub.enum || sub.const)
		)
		if (enumSub?.enum) {
			return enumSub.enum.map((val: any) => ({
				label: String(val).toUpperCase(),
				value: val
			}))
		}
	}
	return []
})

// Auto-populate default value if provided or if field is required with only 1 option
watchEffect(() => {
	const defaultVal = control.value.uischema?.options?.defaultValue
	if (
		defaultVal !== undefined &&
		(control.value.data === undefined ||
			control.value.data === null ||
			control.value.data === '')
	) {
		handleChange(control.value.path, defaultVal)
	} else if (
		selectOptions.value.length === 1 &&
		control.value.required &&
		(control.value.data === undefined ||
			control.value.data === null ||
			control.value.data === '')
	) {
		handleChange(control.value.path, selectOptions.value[0].value)
	}
})

const onSelect = (val: any) => {
	let finalVal = val
	if (
		(val === '' || val === undefined) &&
		control.value.uischema?.options?.emptyPolicy === 'null'
	) {
		finalVal = null
	}
	handleChange(control.value.path, finalVal)
}
</script>

<template>
	<div
		v-if="control.visible"
		:data-invalid="!!control.errors"
		:class="{ 'has-error': !!control.errors }"
	>
		<AppInputSelect
			:model-value="control.data"
			:label="control.label"
			:placeholder="control.uischema?.options?.placeholder || `Select ${control.label}`"
			:required="control.required"
			:error="control.errors"
			:disabled="!control.enabled"
			:options="selectOptions"
			@update:model-value="onSelect"
		/>
	</div>
</template>

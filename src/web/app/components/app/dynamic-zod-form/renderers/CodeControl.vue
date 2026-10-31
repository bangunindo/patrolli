<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed, inject, type ComputedRef } from 'vue'
import { rendererProps, useJsonFormsControl } from '@jsonforms/vue'
import type { ControlElement } from '@jsonforms/core'
import AppInputCode from '@/components/app/input/AppInputCode.vue'
import CustomSlotRenderer from './CustomSlotRenderer.vue'

const props = defineProps({
	...rendererProps<ControlElement>()
})

const { control, handleChange } = useJsonFormsControl(props)
const slots = inject<Record<string, any>>('dynamicZodFormSlots', {})
const formIssues = inject<ComputedRef<Record<string, string>> | null>('formIssues', null)

const leafName = computed(() => {
	const p = control.value.path || ''
	const parts = p.split('.')
	return parts[parts.length - 1] || p
})

const customSlot = computed(() => {
	const pathKey = control.value.path
	return slots[`field:${pathKey}`] || slots[`field:${leafName.value}`] || null
})

const language = computed(() => {
	const opts = control.value.uischema?.options || {}
	if (opts.language) return opts.language
	if (opts.widget === 'opl') return 'opl'
	return 'opl'
})

const fieldError = computed(() => {
	if (control.value.errors) return control.value.errors
	if (formIssues?.value) {
		return formIssues.value[control.value.path] || formIssues.value[leafName.value] || ''
	}
	return ''
})

const onInput = (val: string) => {
	handleChange(control.value.path, val)
}

const slotProps = computed(() => ({
	value: control.value.data,
	modelValue: control.value.data,
	'model-value': control.value.data,
	error: fieldError.value,
	updateModelValue: onInput,
	'update-model-value': onInput,
	handleChange: onInput,
	'handle-change': onInput
}))
</script>

<template>
	<div
		v-if="control.visible"
		:data-invalid="!!fieldError"
		:class="{ 'has-error': !!fieldError, contents: !!customSlot }"
	>
		<!-- Custom Slot Fallback (e.g. #field:config.config.opl) -->
		<CustomSlotRenderer v-if="customSlot" :render-fn="customSlot" :slot-props="slotProps" />

		<!-- Default Code Control -->
		<ClientOnly v-else>
			<AppInputCode
				:model-value="control.data ?? ''"
				:language="language"
				:label="control.label"
				:height="control.uischema?.options?.height || '280px'"
				:placeholder="control.uischema?.options?.placeholder || ''"
				:required="control.required"
				:error="fieldError"
				:disabled="!control.enabled"
				@update:model-value="onInput"
			/>
		</ClientOnly>
	</div>
</template>

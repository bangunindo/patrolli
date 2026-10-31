<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed, inject, type ComputedRef } from 'vue'
import { rendererProps, useJsonFormsControl, DispatchRenderer } from '@jsonforms/vue'
import type { ControlElement, JsonSchema, UISchemaElement } from '@jsonforms/core'
import FieldGroup from '@/components/app/input/FieldGroup.vue'
import CustomSlotRenderer from './CustomSlotRenderer.vue'
import { createControlElement, resolveHintForPath } from '@/app/services/forms/hints-to-uischema'
import type { UiHint } from '@@/shared/types/dynamic-form.type'
import { formatFieldLabel } from '@/app/services/forms/path-helper'

const props = defineProps({
	...rendererProps<ControlElement>()
})

const { control, handleChange } = useJsonFormsControl(props)
const slots = inject<Record<string, any>>('dynamicZodFormSlots', {})
const jsonforms = inject<any>('jsonforms', undefined)
const formHintsRef = inject<ComputedRef<Record<string, UiHint>> | undefined>('formHints', undefined)
const formHints = computed(() => formHintsRef?.value || {})

const leafName = computed(() => {
	const p = control.value.path || ''
	const parts = p.split('.')
	return parts[parts.length - 1] || p
})

const customSlot = computed(() => {
	const pathKey = control.value.path
	const rootData = (control.value as any).rootData ?? jsonforms?.core?.data
	const activeType =
		rootData?.config?.module || rootData?.config?.type || rootData?.module || rootData?.type

	if (activeType && pathKey) {
		const parts = pathKey.split('.')
		if (parts[0] === 'config' && parts.length > 1) {
			const branchSlotKey = ['config', activeType, ...parts.slice(1)].join('.')
			if (slots[`field:${branchSlotKey}`]) {
				return slots[`field:${branchSlotKey}`]
			}
		}
	}

	return slots[`field:${pathKey}`] || slots[`field:${leafName.value}`] || null
})

const onInput = (val: any) => {
	handleChange(control.value.path, val)
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

const schema = computed<JsonSchema>(() => control.value.schema || {})
const properties = computed(() => (schema.value as any).properties || {})

const childUiSchema = computed<UISchemaElement>(() => {
	const elements: ControlElement[] = []
	const basePath = control.value.path || ''

	for (const propName of Object.keys(properties.value)) {
		const fullPath = basePath ? `${basePath}.${propName}` : propName
		const ctrl = createControlElement(
			propName,
			fullPath,
			formHints.value,
			(control.value as any).rootData
		)
		elements.push(ctrl)
	}
	return {
		type: 'VerticalLayout',
		elements
	} as any
})

const selfHint = computed(() => {
	return resolveHintForPath(control.value.path, formHints.value, (control.value as any).rootData)
})

const hideGroup = computed(() => {
	if (control.value.uischema?.options?.hideGroup !== undefined) {
		return control.value.uischema.options.hideGroup === true
	}
	if (selfHint.value?.hideGroup !== undefined) {
		return selfHint.value.hideGroup === true
	}
	return false
})

const groupLabel = computed(() => {
	if (selfHint.value?.group) return selfHint.value.group
	if (selfHint.value?.label) return selfHint.value.label
	if (control.value.label) return control.value.label
	const parts = (control.value.path || '').split('.')
	const leaf = parts[parts.length - 1]
	return leaf ? formatFieldLabel(leaf) : ''
})
</script>

<template>
	<div v-if="control.visible" class="w-full">
		<!-- Custom Slot Fallback (e.g. #field:config.dummy.config or #field:config.config) -->
		<CustomSlotRenderer v-if="customSlot" :render-fn="customSlot" :slot-props="slotProps" />

		<FieldGroup v-else-if="!hideGroup && groupLabel" :label="groupLabel">
			<div class="flex flex-col gap-3">
				<dispatch-renderer
					:schema="schema"
					:uischema="childUiSchema"
					:path="control.path"
					:enabled="control.enabled"
					:renderers="control.renderers"
				/>
			</div>
		</FieldGroup>

		<div v-else class="flex flex-col gap-3">
			<dispatch-renderer
				:schema="schema"
				:uischema="childUiSchema"
				:path="control.path"
				:enabled="control.enabled"
				:renderers="control.renderers"
			/>
		</div>
	</div>
</template>

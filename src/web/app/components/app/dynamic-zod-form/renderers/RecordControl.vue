<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed, inject, type ComputedRef } from 'vue'
import { rendererProps, useJsonFormsControl } from '@jsonforms/vue'
import type { ControlElement } from '@jsonforms/core'
import AppInputRecord from '@/components/app/input/Record.vue'
import { resolveHintForPath } from '@/app/services/forms/hints-to-uischema'
import type { UiHint, FieldDescriptor } from '@@/shared/types/dynamic-form.type'

const props = defineProps({
	...rendererProps<ControlElement>()
})

const { control, handleChange } = useJsonFormsControl(props)
const formHintsRef = inject<ComputedRef<Record<string, UiHint>> | undefined>('formHints', undefined)
const formHints = computed(() => formHintsRef?.value || {})

const selfHint = computed(() => {
	return resolveHintForPath(control.value.path, formHints.value, (control.value as any).rootData)
})

// Synthesize a FieldDescriptor for DynamicFieldsRecord
const descriptor = computed<FieldDescriptor>(() => {
	const schema = (control.value.schema || {}) as any
	const hint = selfHint.value || {}

	// Determine inner child structure from schema.additionalProperties
	const addProps = schema.additionalProperties || {}
	let children: FieldDescriptor[] = []

	if (addProps.type === 'array') {
		children = [
			{
				name: 'value',
				path: ['value'],
				pathKey: 'value',
				label: 'Value',
				widget: 'array-string',
				zodType: 'ZodArray',
				required: false,
				isOptional: true,
				isNullable: false
			}
		]
	} else if (addProps.type === 'object') {
		const innerAddProps = addProps.additionalProperties || {}
		let innerChildren: FieldDescriptor[] = []

		if (innerAddProps.anyOf || innerAddProps.oneOf) {
			// e.g. TransformValue discriminated union (bool, double, int, str)
			innerChildren = [
				{
					name: 'value',
					path: ['value'],
					pathKey: 'value',
					label: 'Value',
					widget: 'discriminated-union',
					zodType: 'ZodUnion',
					required: true,
					isOptional: false,
					isNullable: false,
					unionDiscriminator: 'type',
					unionBranches: {
						bool: [],
						double: [],
						int: [],
						str: []
					}
				}
			]
		} else {
			innerChildren = [
				{
					name: 'value',
					path: ['value'],
					pathKey: 'value',
					label: 'Value',
					widget: 'text',
					zodType: 'ZodString',
					required: false,
					isOptional: true,
					isNullable: false
				}
			]
		}

		children = [
			{
				name: 'value',
				path: ['value'],
				pathKey: 'value',
				label: 'Value',
				widget: 'record',
				zodType: 'ZodRecord',
				required: false,
				isOptional: true,
				isNullable: false,
				children: innerChildren
			}
		]
	}

	return {
		name: (control.value.path || '').split('.').pop() || '',
		path: (control.value.path || '').split('.'),
		pathKey: control.value.path || '',
		label: control.value.label || hint.label || '',
		widget: 'record',
		zodType: 'ZodRecord',
		required: control.value.required,
		isOptional: !control.value.required,
		isNullable: false,
		options: hint.keyOptions || hint.options || control.value.uischema?.options?.keyOptions,
		hints: hint,
		children
	}
})

const onUpdate = (val: any) => {
	handleChange(control.value.path, val)
}
</script>

<template>
	<div
		v-if="control.visible"
		:data-invalid="!!control.errors"
		:class="{ 'has-error': !!control.errors }"
		class="w-full"
	>
		<AppInputRecord
			:model-value="control.data"
			:descriptor="descriptor"
			:hints="formHints"
			:error="control.errors"
			:disabled="!control.enabled"
			@update:model-value="onUpdate"
		/>
	</div>
</template>

<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed, inject, type ComputedRef } from 'vue'
import { rendererProps, useJsonFormsControl, DispatchRenderer } from '@jsonforms/vue'
import type { ControlElement, UISchemaElement } from '@jsonforms/core'
import AppInputText from '@/components/app/input/AppInputText.vue'
import AppInputError from '@/components/app/input/AppInputError.vue'
import CustomSlotRenderer from './CustomSlotRenderer.vue'
import { createControlElement, resolveHintForPath } from '@/app/services/forms/hints-to-uischema'
import type { UiHint } from '@@/shared/types/dynamic-form.type'
import { formatFieldLabel } from '@/app/services/forms/path-helper'

const props = defineProps({
	...rendererProps<ControlElement>()
})

const { control, handleChange } = useJsonFormsControl(props)
const slots = inject<Record<string, any>>('dynamicZodFormSlots', {})
const formHintsRef = inject<ComputedRef<Record<string, UiHint>> | undefined>('formHints', undefined)
const formHints = computed(() => formHintsRef?.value || {})

const leafName = computed(() => {
	const p = control.value.path || ''
	const parts = p.split('.')
	return parts[parts.length - 1] || p
})

const selfHint = computed(() => {
	return resolveHintForPath(control.value.path, formHints.value, (control.value as any).rootData)
})

const fieldLabel = computed(() => {
	if (selfHint.value?.label) return selfHint.value.label
	if (control.value.label) return control.value.label
	return leafName.value ? formatFieldLabel(leafName.value) : ''
})

const placeholder = computed(() => {
	return (control.value.uischema?.options?.placeholder ||
		selfHint.value?.placeholder ||
		'') as string
})

const customSlot = computed(() => {
	const pathKey = control.value.path
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

const items = computed<any[]>(() => {
	return Array.isArray(control.value.data) ? control.value.data : []
})

const itemsSchema = computed<any>(() => {
	return (control.value.schema as any)?.items || {}
})

const isPrimitive = computed(() => {
	const type = itemsSchema.value.type
	if (type === 'string' || type === 'number' || type === 'integer' || type === 'boolean') {
		return true
	}
	if (
		!type &&
		!itemsSchema.value.properties &&
		!itemsSchema.value.anyOf &&
		!itemsSchema.value.oneOf
	) {
		return true
	}
	return false
})

// Primitive Array Handlers
const updatePrimitiveItem = (idx: number, val: string | number) => {
	const current = [...items.value]
	current[idx] =
		itemsSchema.value.type === 'number' || itemsSchema.value.type === 'integer'
			? Number(val)
			: String(val)
	handleChange(control.value.path, current)
}

const addPrimitiveItem = () => {
	if (!control.value.enabled) return
	const current = [...items.value]
	const defVal =
		itemsSchema.value.type === 'number' || itemsSchema.value.type === 'integer' ? 0 : ''
	current.push(defVal)
	handleChange(control.value.path, current)
}

const removePrimitiveItem = (idx: number) => {
	if (!control.value.enabled) return
	const current = [...items.value]
	current.splice(idx, 1)
	handleChange(control.value.path, current)
}

// Object Array Handlers
const createDefaultItemFromSchema = (schema: any): any => {
	if (!schema) return {}
	if (schema.default !== undefined) return structuredClone(schema.default)
	if (schema.type === 'string') return ''
	if (schema.type === 'number' || schema.type === 'integer') return 0
	if (schema.type === 'boolean') return false
	if (schema.type === 'array') return []
	if (schema.type === 'object' || schema.properties) {
		const obj: Record<string, any> = {}
		const props = schema.properties || {}
		for (const [key, propSchema] of Object.entries<any>(props)) {
			if (propSchema.enum && propSchema.enum.length === 1) {
				obj[key] = propSchema.enum[0]
			} else if (propSchema.default !== undefined) {
				obj[key] = structuredClone(propSchema.default)
			} else if (propSchema.type === 'string') {
				obj[key] = ''
			} else if (propSchema.type === 'number' || propSchema.type === 'integer') {
				obj[key] = 0
			} else if (propSchema.type === 'boolean') {
				obj[key] = false
			} else if (propSchema.type === 'array') {
				obj[key] = []
			} else if (propSchema.type === 'object' || propSchema.properties) {
				obj[key] = createDefaultItemFromSchema(propSchema)
			}
		}
		return obj
	}
	return {}
}

const addObjectItem = () => {
	if (!control.value.enabled) return
	const current = [...items.value]
	const newItem = createDefaultItemFromSchema(itemsSchema.value)
	current.push(newItem)
	handleChange(control.value.path, current)
}

const removeObjectItem = (idx: number) => {
	if (!control.value.enabled) return
	const current = [...items.value]
	current.splice(idx, 1)
	handleChange(control.value.path, current)
}

const getItemUiSchema = (idx: number): UISchemaElement => {
	if (itemsSchema.value.properties) {
		const props = itemsSchema.value.properties
		const elements: ControlElement[] = []
		const itemPath = `${control.value.path}.${idx}`
		for (const propName of Object.keys(props)) {
			const fullPath = `${itemPath}.${propName}`
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
	}
	return {
		type: 'Control',
		scope: '#'
	}
}
</script>

<template>
	<div
		v-if="control.visible"
		:data-invalid="!!control.errors"
		:class="{ 'has-error': !!control.errors }"
		class="w-full"
	>
		<!-- Custom Slot Fallback (e.g. #field:input_items, #field:input_pipelines) -->
		<CustomSlotRenderer v-if="customSlot" :render-fn="customSlot" :slot-props="slotProps" />

		<!-- Primitive Array Mode (string, number, etc.) -->
		<div v-else-if="isPrimitive" class="flex flex-col gap-2 p-1">
			<label v-if="fieldLabel" class="font-semibold block text-sm">
				{{ fieldLabel }}
				<span v-if="control.required" class="text-destructive">*</span>
			</label>

			<div v-for="(_item, idx) in items" :key="idx" class="flex items-center gap-2">
				<AppInputText
					:model-value="items[idx]"
					class="flex-1"
					:placeholder="placeholder || `Item #${idx + 1}`"
					:disabled="!control.enabled"
					@update:model-value="updatePrimitiveItem(idx, $event)"
				/>
				<UiButton
					type="button"
					variant="outline"
					size="icon"
					class="h-8 w-8 text-destructive border-destructive/30 hover:bg-destructive/10 shrink-0"
					:disabled="!control.enabled"
					@click="removePrimitiveItem(idx)"
				>
					<UiIcon name="minus" size="14px" />
				</UiButton>
			</div>

			<div>
				<button
					v-if="control.enabled"
					type="button"
					class="text-xs font-semibold text-primary hover:underline flex items-center gap-1 cursor-pointer"
					@click="addPrimitiveItem"
				>
					<UiIcon name="plus" size="14px" />
					<span>Add {{ fieldLabel }}</span>
				</button>
			</div>

			<AppInputError :messages="control.errors" />
		</div>

		<!-- Object Array Mode -->
		<div v-else class="flex flex-col gap-2 p-2 border border-border rounded-lg bg-card">
			<div class="flex justify-between items-center">
				<label v-if="fieldLabel" class="font-semibold text-sm text-foreground">
					{{ fieldLabel }}
					<span v-if="control.required" class="text-destructive">*</span>
				</label>
				<span class="text-xs text-muted-foreground">
					{{ items.length }} item{{ items.length !== 1 ? 's' : '' }}
				</span>
			</div>

			<div
				v-for="(_item, idx) in items"
				:key="idx"
				class="flex items-start gap-2 p-3 bg-muted/30 rounded-md border border-border"
			>
				<div class="flex-1">
					<dispatch-renderer
						:schema="itemsSchema"
						:uischema="getItemUiSchema(idx)"
						:path="`${control.path}.${idx}`"
						:enabled="control.enabled"
						:renderers="control.renderers"
					/>
				</div>
				<UiButton
					type="button"
					variant="outline"
					size="icon"
					class="h-8 w-8 text-destructive border-destructive/30 hover:bg-destructive/10 shrink-0"
					:disabled="!control.enabled"
					@click="removeObjectItem(idx)"
				>
					<UiIcon name="minus" size="14px" />
				</UiButton>
			</div>

			<div>
				<button
					v-if="control.enabled"
					type="button"
					class="text-xs font-semibold text-primary hover:underline flex items-center gap-1 cursor-pointer"
					@click="addObjectItem"
				>
					<UiIcon name="plus" size="14px" />
					<span>Add {{ fieldLabel }}</span>
				</button>
			</div>

			<AppInputError :messages="control.errors" />
		</div>
	</div>
</template>

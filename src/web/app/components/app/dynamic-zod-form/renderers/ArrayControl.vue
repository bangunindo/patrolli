<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed, watch } from 'vue'
import type { FieldDescriptor } from '@@/shared/types/dynamic-form.type'
import AppInputText from '@/components/app/input/AppInputText.vue'
import AppInputError from '@/components/app/input/AppInputError.vue'
import CustomSlotRenderer from './CustomSlotRenderer.vue'
import DynamicFieldRenderer from '../DynamicFieldRenderer.vue'
import { formatFieldLabel, safeClone } from '@/app/services/forms/path-helper'
import { useDynamicField } from '../useDynamicField'

const props = defineProps<{
	descriptor: FieldDescriptor
	path: string
}>()

const {
	ctx,
	fieldApi,
	value,
	setValue,
	error,
	hint,
	label,
	required,
	disabled,
	leafName,
	customSlot,
	slotProps
} = useDynamicField(props, { mode: 'array' })

const arrayFieldApi = fieldApi as any

const fieldLabel = computed(() => {
	if (hint.value.label) return hint.value.label
	if (label.value) return label.value
	return leafName.value ? formatFieldLabel(leafName.value) : ''
})

const placeholder = computed(() => {
	return (hint.value.placeholder || props.descriptor.placeholder || '') as string
})

const items = computed<any[]>(() => {
	return Array.isArray(value.value) ? value.value : []
})

const hasItems = computed(() => items.value.length > 0)

const childDescriptors = computed<FieldDescriptor[]>(() => {
	if (props.descriptor.children && props.descriptor.children.length > 0) {
		return props.descriptor.children
	}
	if (typeof props.descriptor.resolveChildren === 'function' && hasItems.value) {
		return props.descriptor.resolveChildren()
	}
	return []
})

const isPrimitive = computed(() => {
	return (
		props.descriptor.widget === 'array-string' ||
		(childDescriptors.value.length === 0 &&
			typeof props.descriptor.resolveChildren !== 'function')
	)
})

let keyCounter = 0
const nextKey = () => `arr-item-${++keyCounter}`
let objectKeys: string[] = []
let primitiveKeys: string[] = []

const getItemKey = (keys: string[], idx: number): string => {
	while (keys.length <= idx) {
		keys.push(nextKey())
	}
	return keys[idx]
}

const getObjectItemKey = (idx: number): string => getItemKey(objectKeys, idx)
const getPrimitiveItemKey = (idx: number): string => getItemKey(primitiveKeys, idx)

watch(
	() => items.value.length,
	(len) => {
		if (primitiveKeys.length > len) {
			primitiveKeys.length = len
		}
		if (objectKeys.length > len) {
			objectKeys.length = len
		}
	}
)

const updatePrimitiveItem = (idx: number, val: string | number) => {
	ctx.form.setFieldValue(`${props.path}.${idx}`, val)
	ctx.syncModelFromForm(props.path)
}

const addPrimitiveItem = () => {
	if (disabled.value) return
	if (!Array.isArray(value.value)) {
		primitiveKeys = [nextKey()]
		setValue([''])
		return
	}
	primitiveKeys.push(nextKey())
	arrayFieldApi.pushValue('')
	ctx.syncModelFromForm(props.path)
}

const removePrimitiveItem = (idx: number) => {
	if (disabled.value) return
	primitiveKeys.splice(idx, 1)
	arrayFieldApi.removeValue(idx)
	ctx.syncModelFromForm(props.path)
}

const createDefaultObject = (children: FieldDescriptor[]): Record<string, any> => {
	const obj: Record<string, any> = {}
	for (const child of children) {
		if (child.defaultValue !== undefined) {
			obj[child.name] = safeClone(child.defaultValue)
		} else if (child.zodType === 'array') {
			obj[child.name] = []
		} else if (child.zodType === 'boolean') {
			obj[child.name] = false
		} else if (child.zodType === 'object' && child.children) {
			obj[child.name] = createDefaultObject(child.children)
		}
	}
	return obj
}

const addObjectItem = () => {
	if (disabled.value) return
	const targetChildren =
		childDescriptors.value.length > 0
			? childDescriptors.value
			: (props.descriptor.resolveChildren?.() ?? [])
	const newObj = createDefaultObject(targetChildren)
	if (!Array.isArray(value.value)) {
		objectKeys = [nextKey()]
		setValue([newObj])
		return
	}
	objectKeys.push(nextKey())
	arrayFieldApi.pushValue(newObj)
	ctx.syncModelFromForm(props.path)
}

const removeObjectItem = (idx: number) => {
	if (disabled.value) return
	objectKeys.splice(idx, 1)
	arrayFieldApi.removeValue(idx)
	ctx.syncModelFromForm(props.path)
}
</script>

<template>
	<div :data-invalid="!!error" :class="{ 'has-error': !!error }" class="w-full">
		<CustomSlotRenderer v-if="customSlot" :render-fn="customSlot" :slot-props="slotProps" />

		<!-- Primitive Array Mode -->
		<div v-else-if="isPrimitive" class="flex flex-col gap-2 p-1">
			<label v-if="fieldLabel" class="font-semibold block text-sm">
				{{ fieldLabel }}
				<span v-if="required" class="text-destructive">*</span>
			</label>

			<div
				v-for="(_item, idx) in items"
				:key="getPrimitiveItemKey(idx)"
				class="flex items-center gap-2"
			>
				<AppInputText
					:model-value="items[idx]"
					class="flex-1"
					:placeholder="placeholder || `Item #${idx + 1}`"
					:disabled="disabled"
					@update:model-value="updatePrimitiveItem(idx, $event)"
				/>
				<UiButton
					type="button"
					variant="outline"
					size="icon"
					class="h-8 w-8 text-destructive border-destructive/30 hover:bg-destructive/10 shrink-0"
					:disabled="disabled"
					@click="removePrimitiveItem(idx)"
				>
					<UiIcon name="minus" size="14px" />
				</UiButton>
			</div>

			<div>
				<button
					v-if="!disabled"
					type="button"
					class="text-xs font-semibold text-primary hover:underline flex items-center gap-1 cursor-pointer"
					@click="addPrimitiveItem"
				>
					<UiIcon name="plus" size="14px" />
					<span>Add {{ fieldLabel }}</span>
				</button>
			</div>

			<AppInputError :error="error" />
		</div>

		<!-- Object Array Mode -->
		<div v-else class="flex flex-col gap-2 p-2 border border-border rounded-lg bg-card">
			<div class="flex justify-between items-center">
				<label v-if="fieldLabel" class="font-semibold text-sm text-foreground">
					{{ fieldLabel }}
					<span v-if="required" class="text-destructive">*</span>
				</label>
				<span class="text-xs text-muted-foreground">
					{{ items.length }} item{{ items.length !== 1 ? 's' : '' }}
				</span>
			</div>

			<div
				v-for="(_item, idx) in items"
				:key="getObjectItemKey(idx)"
				class="flex items-start gap-2 p-3 bg-muted/30 rounded-md border border-border"
			>
				<div class="flex-1 flex flex-col gap-3">
					<DynamicFieldRenderer
						v-for="child in childDescriptors"
						:key="`${path}.${idx}.${child.name}`"
						:descriptor="child"
						:path="`${path}.${idx}.${child.name}`"
					/>
				</div>
				<UiButton
					type="button"
					variant="outline"
					size="icon"
					class="h-8 w-8 text-destructive border-destructive/30 hover:bg-destructive/10 shrink-0"
					:disabled="disabled"
					@click="removeObjectItem(idx)"
				>
					<UiIcon name="minus" size="14px" />
				</UiButton>
			</div>

			<div>
				<button
					v-if="!disabled"
					type="button"
					class="text-xs font-semibold text-primary hover:underline flex items-center gap-1 cursor-pointer"
					@click="addObjectItem"
				>
					<UiIcon name="plus" size="14px" />
					<span>Add {{ fieldLabel }}</span>
				</button>
			</div>

			<AppInputError :error="error" />
		</div>
	</div>
</template>

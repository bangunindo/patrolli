<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed } from 'vue'
import type { FieldDescriptor } from '@@/shared/types/dynamic-form.type'
import AppInputError from '@/components/app/input/AppInputError.vue'
import CustomSlotRenderer from './CustomSlotRenderer.vue'
import { useDynamicField } from '../useDynamicField'

const props = defineProps<{
	descriptor: FieldDescriptor
	path: string
}>()

const { value, setValue, error, hint, label, required, disabled, customSlot, slotProps } =
	useDynamicField(props)

const isChecked = computed(() => {
	if (typeof value.value === 'number') {
		return value.value === 1
	}
	return Boolean(value.value)
})

const isBooleanText = computed(
	() => props.descriptor.zodType === 'boolean' && hint.value.widget !== 'switch'
)

const statusLabel = computed(() => {
	if (isBooleanText.value) {
		return isChecked.value ? 'Yes' : 'No'
	}
	return isChecked.value ? 'Active' : 'Inactive'
})

const onToggle = (val: boolean) => {
	const isNumber = props.descriptor.zodType === 'number' || props.descriptor.zodType === 'int'
	setValue(isNumber ? (val ? 1 : 0) : val)
}
</script>

<template>
	<div :data-invalid="!!error" :class="{ 'has-error': !!error }">
		<CustomSlotRenderer v-if="customSlot" :render-fn="customSlot" :slot-props="slotProps" />

		<div v-else class="flex flex-col gap-1 p-1">
			<label class="font-semibold block text-sm text-foreground">
				{{ label }}
				<span v-if="required" class="text-destructive">*</span>
			</label>
			<div class="flex items-center gap-2 pt-1">
				<UiSwitch :checked="isChecked" :disabled="disabled" @update:checked="onToggle" />
				<span class="text-sm font-normal text-muted-foreground">
					{{ statusLabel }}
				</span>
			</div>
			<AppInputError :error="error" />
		</div>
	</div>
</template>

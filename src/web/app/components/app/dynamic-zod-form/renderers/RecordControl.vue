<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed } from 'vue'
import type { FieldDescriptor } from '@@/shared/types/dynamic-form.type'
import AppInputRecord from '@/components/app/input/Record.vue'
import CustomSlotRenderer from './CustomSlotRenderer.vue'
import { useDynamicField } from '../useDynamicField'

const props = defineProps<{
	descriptor: FieldDescriptor
	path: string
}>()

const { ctx, value, setValue, error, hint, label, disabled, customSlot, slotProps } =
	useDynamicField(props)

const recordDescriptor = computed<FieldDescriptor>(() => {
	const resolvedOptions = Array.isArray(hint.value.keyOptions)
		? hint.value.keyOptions
		: Array.isArray(hint.value.options)
			? hint.value.options
			: props.descriptor.options
	return {
		...props.descriptor,
		label: label.value || props.descriptor.label,
		options: resolvedOptions,
		hints: hint.value
	}
})
</script>

<template>
	<div :data-invalid="!!error" :class="{ 'has-error': !!error }" class="w-full">
		<CustomSlotRenderer v-if="customSlot" :render-fn="customSlot" :slot-props="slotProps" />

		<AppInputRecord
			v-else
			:model-value="value"
			:descriptor="recordDescriptor"
			:hints="ctx.formHints.value"
			:error="error"
			:disabled="disabled"
			@update:model-value="setValue"
		/>
	</div>
</template>

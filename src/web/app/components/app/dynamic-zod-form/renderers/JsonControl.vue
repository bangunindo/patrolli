<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import type { FieldDescriptor } from '@@/shared/types/dynamic-form.type'
import AppInputJson from '@/components/app/input/Json.vue'
import CustomSlotRenderer from './CustomSlotRenderer.vue'
import { useDynamicField } from '../useDynamicField'

const props = defineProps<{
	descriptor: FieldDescriptor
	path: string
}>()

const { value, setValue, error, hint, label, required, disabled, customSlot, slotProps } =
	useDynamicField(props)
</script>

<template>
	<div :data-invalid="!!error" :class="{ 'has-error': !!error, contents: !!customSlot }">
		<CustomSlotRenderer v-if="customSlot" :render-fn="customSlot" :slot-props="slotProps" />

		<ClientOnly v-else>
			<AppInputJson
				:model-value="value"
				:label="label || 'JSON'"
				:height="(hint as Record<string, any>).height || '250px'"
				:required="required"
				:error="error"
				:disabled="disabled"
				@update:model-value="setValue"
			/>
		</ClientOnly>
	</div>
</template>

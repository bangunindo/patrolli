<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed } from 'vue'
import type { FieldDescriptor } from '@@/shared/types/dynamic-form.type'
import AppInputCode from '@/components/app/input/AppInputCode.vue'
import CustomSlotRenderer from './CustomSlotRenderer.vue'
import { useDynamicField } from '../useDynamicField'

const props = defineProps<{
	descriptor: FieldDescriptor
	path: string
}>()

const { value, setValue, error, hint, label, required, disabled, customSlot, slotProps } =
	useDynamicField(props)

const language = computed(() => {
	const h = hint.value as Record<string, any>
	if (h.language) return h.language
	if (h.widget === 'opl') return 'opl'
	return 'opl'
})
</script>

<template>
	<div :data-invalid="!!error" :class="{ 'has-error': !!error, contents: !!customSlot }">
		<CustomSlotRenderer v-if="customSlot" :render-fn="customSlot" :slot-props="slotProps" />

		<ClientOnly v-else>
			<AppInputCode
				:model-value="typeof value === 'string' ? value : ''"
				:language="language"
				:label="label"
				:height="(hint as Record<string, any>).height || '280px'"
				:placeholder="hint.placeholder || descriptor.placeholder || ''"
				:required="required"
				:error="error"
				:disabled="disabled"
				@update:model-value="(val: string) => setValue(val)"
			/>
		</ClientOnly>
	</div>
</template>

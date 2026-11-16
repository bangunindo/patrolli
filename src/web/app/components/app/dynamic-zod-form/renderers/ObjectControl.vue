<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed } from 'vue'
import type { FieldDescriptor } from '@@/shared/types/dynamic-form.type'
import FieldGroup from '@/components/app/input/FieldGroup.vue'
import CustomSlotRenderer from './CustomSlotRenderer.vue'
import DynamicFieldRenderer from '../DynamicFieldRenderer.vue'
import { formatFieldLabel } from '@/app/services/forms/path-helper'
import { useDynamicField } from '../useDynamicField'

const props = defineProps<{
	descriptor: FieldDescriptor
	path: string
}>()

const { hint, label, leafName, customSlot, slotProps } = useDynamicField(props)

const children = computed(() => props.descriptor.children || [])

const buildChildPath = (child: FieldDescriptor) => {
	return props.path ? `${props.path}.${child.name}` : child.name
}

const hideGroup = computed(() => {
	if (hint.value.hideGroup !== undefined) {
		return hint.value.hideGroup === true
	}
	return false
})

const groupLabel = computed(() => {
	if (hint.value.group) return hint.value.group
	if (hint.value.label) return hint.value.label
	if (label.value) return label.value
	return leafName.value ? formatFieldLabel(leafName.value) : ''
})
</script>

<template>
	<div class="w-full">
		<CustomSlotRenderer v-if="customSlot" :render-fn="customSlot" :slot-props="slotProps" />

		<FieldGroup v-else-if="!hideGroup && groupLabel" :label="groupLabel">
			<div class="flex flex-col gap-3">
				<DynamicFieldRenderer
					v-for="child in children"
					:key="buildChildPath(child)"
					:descriptor="child"
					:path="buildChildPath(child)"
				/>
			</div>
		</FieldGroup>

		<div v-else class="flex flex-col gap-3">
			<DynamicFieldRenderer
				v-for="child in children"
				:key="buildChildPath(child)"
				:descriptor="child"
				:path="buildChildPath(child)"
			/>
		</div>
	</div>
</template>

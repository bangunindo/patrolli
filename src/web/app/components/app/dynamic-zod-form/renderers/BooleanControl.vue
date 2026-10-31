<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed } from 'vue'
import { rendererProps, useJsonFormsControl } from '@jsonforms/vue'
import type { ControlElement } from '@jsonforms/core'
import AppInputError from '@/components/app/input/AppInputError.vue'

const props = defineProps({
	...rendererProps<ControlElement>()
})

const { control, handleChange } = useJsonFormsControl(props)

const isChecked = computed(() => {
	return Boolean(control.value.data)
})

const onToggle = (val: boolean) => {
	handleChange(control.value.path, val)
}
</script>

<template>
	<div
		v-if="control.visible"
		class="flex flex-col gap-1 p-1"
		:data-invalid="!!control.errors"
		:class="{ 'has-error': !!control.errors }"
	>
		<label class="font-semibold block text-sm text-foreground">
			{{ control.label }}
			<span v-if="control.required" class="text-destructive">*</span>
		</label>
		<div class="flex items-center gap-2 pt-1">
			<UiSwitch
				:checked="isChecked"
				:disabled="!control.enabled"
				@update:checked="onToggle"
			/>
			<span class="text-sm font-normal text-muted-foreground">
				{{ isChecked ? 'Yes' : 'No' }}
			</span>
		</div>
		<AppInputError :error="control.errors" />
	</div>
</template>

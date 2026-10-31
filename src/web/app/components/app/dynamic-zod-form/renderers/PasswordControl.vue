<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { rendererProps, useJsonFormsControl } from '@jsonforms/vue'
import type { ControlElement } from '@jsonforms/core'
import Password from '@/components/app/input/Password.vue'

const props = defineProps({
	...rendererProps<ControlElement>()
})

const { control, handleChange } = useJsonFormsControl(props)

const onInput = (val: string | number) => {
	handleChange(control.value.path, val)
}
</script>

<template>
	<div
		v-if="control.visible"
		:data-invalid="!!control.errors"
		:class="{ 'has-error': !!control.errors }"
	>
		<Password
			:model-value="control.data ?? ''"
			:label="control.label"
			:placeholder="control.uischema?.options?.placeholder || `Enter ${control.label}`"
			:required="control.required"
			:error="control.errors"
			:disabled="!control.enabled"
			@update:model-value="onInput"
		/>
	</div>
</template>

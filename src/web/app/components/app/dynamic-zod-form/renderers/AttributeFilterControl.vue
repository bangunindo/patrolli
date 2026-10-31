<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed, inject, type ComputedRef } from 'vue'
import { rendererProps, useJsonFormsControl } from '@jsonforms/vue'
import type { ControlElement } from '@jsonforms/core'
import type { UiHint } from '@@/shared/types/dynamic-form.type'

const props = defineProps({
	...rendererProps<ControlElement>()
})

const { control, handleChange } = useJsonFormsControl(props)
const formHintsRef = inject<ComputedRef<Record<string, UiHint>> | undefined>('formHints', undefined)
const formHints = computed(() => formHintsRef?.value || {})

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
		<AppInputAttributeFilter
			:model-value="control.data"
			:hints="formHints"
			:error="control.errors"
			:disabled="!control.enabled"
			:root="true"
			@update:model-value="onUpdate"
		/>
	</div>
</template>

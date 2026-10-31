<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import appHelper from '@/app/helpers/app.helper'
import { computed, watch } from 'vue'
import AppInputSelect from '@/components/app/input/AppInputSelect.vue'

interface Props {
	label?: string
	labelClass?: string
	placeholder?: string
	multiple?: boolean
	required?: boolean
	disabled?: boolean
	clearable?: boolean
	error?: string | string[]
	rules?: string[]
}

const props = withDefaults(defineProps<Props>(), {
	label: undefined,
	labelClass: '',
	placeholder: 'Select Cluster',
	multiple: false,
	required: false,
	disabled: false,
	clearable: false,
	error: '',
	rules: () => []
})

const emits = defineEmits<{
	close: []
	'update:error': [value: string]
}>()

const modelValue = defineModel<any>()

const options = computed(() => appHelper.getBaseURLs())

watch(
	options,
	(newOptions) => {
		if (newOptions && newOptions.length === 1 && newOptions[0]?.value) {
			if (props.multiple) {
				if (
					!Array.isArray(modelValue.value) ||
					!modelValue.value.includes(newOptions[0].value)
				) {
					modelValue.value = [newOptions[0].value]
				}
			} else {
				if (!modelValue.value || modelValue.value !== newOptions[0].value) {
					modelValue.value = newOptions[0].value
				}
			}
		}
	},
	{ immediate: true, deep: true }
)
</script>

<template>
	<AppInputSelect
		v-model="modelValue"
		:options="options"
		:label="props.label"
		:label-class="props.labelClass"
		:placeholder="props.placeholder"
		:multiple="props.multiple"
		:required="props.required"
		:disabled="props.disabled"
		:clearable="props.clearable"
		:error="props.error"
		:rules="props.rules"
		label-key="label"
		value-key="value"
		@close="emits('close')"
		@update:error="emits('update:error', $event)"
	/>
</template>

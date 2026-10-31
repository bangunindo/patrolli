<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed } from 'vue'
import AppInputError from '@/components/app/input/AppInputError.vue'
import AppInputText from '@/components/app/input/AppInputText.vue'
import { useValidation } from '@/composables/useValidation'

const props = withDefaults(
	defineProps<{
		error?: string
		label?: string
		placeholder?: string
		required?: boolean
		disabled?: boolean
		rules?: string[]
	}>(),
	{
		error: '',
		label: 'Label',
		placeholder: 'Input Data',
		required: false,
		disabled: false
	}
)

const emits = defineEmits<{
	close: []
	'update:error': [value: string]
}>()

const inputs = defineModel<string[]>({
	default: () => []
})

const addItem = () => {
	if (props.disabled) return
	inputs.value.push('')
	resetFormSubmitState()
}

const removeItem = (index: number) => {
	if (props.disabled) return
	inputs.value.splice(index, 1)
}

const rules = computed(() => {
	const r: string[] = [...(props.rules || [])]
	if (props.required && !r.includes('array')) r.push('array')
	return r
})

const { elRef, errorMessage, showMessages, resetFormSubmitState, handleAlertClose } = useValidation(
	props.label,
	inputs,
	rules,
	{
		error: () => props.error,
		onErrorChange: (val) => emits('update:error', val),
		onClose: () => emits('close')
	}
)
</script>

<template>
	<div ref="elRef" class="p-1 space-y-1.5">
		<label
			v-if="props.label"
			class="text-sm font-semibold text-foreground flex items-center gap-1"
		>
			<span>{{ props.label }}</span>
			<span v-if="props.required" class="text-destructive">*</span>
		</label>

		<div v-for="(input, index) in inputs" :key="index" class="flex items-start gap-2 pt-1">
			<AppInputText
				v-model="inputs[index]"
				type="text"
				:placeholder="props.placeholder"
				class="flex-1 min-w-[120px]"
				:required="props.required"
				:disabled="props.disabled"
			/>
			<UiButton
				type="button"
				variant="outline"
				size="icon"
				class="h-9 w-9 text-destructive border-destructive/30 hover:bg-destructive/10 shrink-0"
				:disabled="props.disabled"
				@click="removeItem(index)"
			>
				<UiIcon name="minus" size="16px" />
			</UiButton>
		</div>

		<button
			v-if="!props.disabled"
			type="button"
			class="mt-2 flex items-center gap-1.5 text-xs font-semibold text-primary hover:underline cursor-pointer"
			@click="addItem"
		>
			<UiIcon name="plus" size="14px" />
			<span>Add {{ props.label }}</span>
		</button>

		<AppInputError v-model="showMessages" :messages="errorMessage" @close="handleAlertClose" />
	</div>
</template>

<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed, ref } from 'vue'
import AppInputError from '@/components/app/input/AppInputError.vue'
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
		label: 'Password',
		placeholder: 'Your password',
		required: false,
		disabled: false
	}
)

const emits = defineEmits<{
	close: []
	'update:error': [value: string]
}>()

const input = defineModel<string | null>({ default: null })
const passwordView = ref(false)

const togglePasswordView = () => {
	passwordView.value = !passwordView.value
}

const rules = computed(() => {
	const r: string[] = [...(props.rules || [])]
	if (props.required && !r.includes('required')) r.push('required')
	return r
})

const { elRef, errorMessage, showMessages, resetFormSubmitState, handleAlertClose } = useValidation(
	props.label,
	input,
	rules,
	{
		error: () => props.error,
		onErrorChange: (val) => emits('update:error', val),
		onClose: () => emits('close')
	}
)
</script>

<template>
	<div ref="elRef" class="w-full space-y-1.5 p-1">
		<label
			v-if="props.label"
			class="text-sm font-semibold text-foreground flex items-center gap-1"
		>
			<span>{{ props.label }}</span>
			<span v-if="props.required" class="text-destructive">*</span>
		</label>

		<div class="relative w-full">
			<UiInput
				v-model="input"
				:type="passwordView ? 'text' : 'password'"
				:placeholder="props.placeholder"
				:required="props.required"
				:disabled="props.disabled"
				:class="[
					'pr-10',
					errorMessage && showMessages
						? 'border-destructive focus-visible:ring-destructive'
						: ''
				]"
				@update:model-value="resetFormSubmitState"
				v-bind="{ ...$attrs }"
			/>
			<UiButton
				type="button"
				variant="ghost"
				size="icon"
				class="h-7 w-7 absolute right-1.5 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
				:title="passwordView ? 'Hide password' : 'Show password'"
				@click="togglePasswordView"
			>
				<UiIcon :name="passwordView ? 'eye-off' : 'eye'" size="16px" />
			</UiButton>
		</div>

		<AppInputError v-model="showMessages" :messages="errorMessage" @close="handleAlertClose" />
	</div>
</template>

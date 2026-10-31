<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed } from 'vue'
import AppInputError from '@/components/app/input/AppInputError.vue'
import { useValidation } from '@/composables/useValidation'
import { cn } from '@/app/lib/utils'
import UiInput from '@/components/ui/input/Input.vue'
import UiIcon from '@/components/ui/icon/Icon.vue'

const props = withDefaults(
	defineProps<{
		error?: string
		label?: string
		labelClass?: string
		placeholder?: string
		type?: string
		required?: boolean
		uuid?: boolean
		rules?: string[]
		step?: string
		size?: string
		preIcon?: string
		readonly?: boolean
		disabled?: boolean
	}>(),
	{
		error: '',
		label: '',
		placeholder: 'Input Data',
		required: false,
		uuid: false,
		size: 'md',
		readonly: false,
		disabled: false
	}
)

const emits = defineEmits<{
	close: []
	'update:error': [value: string]
}>()

const model = defineModel<any>(undefined)

const rules = computed(() => {
	const r: string[] = [...(props.rules || [])]
	if (props.required && !r.includes('required')) r.push('required')
	if (props.uuid && !r.includes('uuid')) r.push('uuid')
	return r
})

const { elRef, errorMessage, showMessages, resetFormSubmitState, handleAlertClose } = useValidation(
	props.label,
	model,
	rules,
	{
		error: () => props.error,
		onErrorChange: (val) => emits('update:error', val),
		onClose: () => emits('close')
	}
)
</script>

<template>
	<div ref="elRef" class="w-full space-y-1.5">
		<label
			v-if="props.label"
			:class="
				cn(
					'text-sm font-semibold text-foreground flex items-center gap-1',
					props.labelClass
				)
			"
		>
			<span>{{ props.label }}</span>
			<span v-if="props.required" class="text-destructive">*</span>
		</label>
		<div class="relative w-full">
			<UiIcon
				v-if="props.preIcon"
				:name="props.preIcon"
				size="16px"
				class="absolute left-3 top-1/2 -translate-y-1/2 text-muted-foreground"
			/>
			<UiInput
				v-model="model"
				:type="props.type"
				:placeholder="props.placeholder"
				:required="props.required"
				:step="props.step"
				:readonly="props.readonly"
				:disabled="props.disabled"
				:class="[
					props.preIcon ? 'pl-9' : '',
					errorMessage && showMessages
						? 'border-destructive focus-visible:ring-destructive'
						: ''
				]"
				@update:model-value="resetFormSubmitState"
				v-bind="{ ...$attrs }"
			/>
		</div>
		<AppInputError v-model="showMessages" :messages="errorMessage" @close="handleAlertClose" />
	</div>
</template>

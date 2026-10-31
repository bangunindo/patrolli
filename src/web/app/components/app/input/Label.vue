<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { watchOnce } from '@vueuse/core'
import { computed, ref } from 'vue'
import AppInputError from '@/components/app/input/AppInputError.vue'
import { useValidation } from '@/composables/useValidation'

type map = Record<string, any>

const emits = defineEmits<{
	'update:modelValue': [val: string]
	'update:error': [val: string]
	close: []
}>()

const props = withDefaults(
	defineProps<{
		modelValue: string
		label?: string
		required?: boolean
		error?: string | string[]
		rules?: string[]
	}>(),
	{
		label: 'Labels',
		required: false,
		error: '',
		rules: () => []
	}
)

const rules = computed(() => {
	const r: string[] = [...(props.rules || [])]
	if (props.required && !r.includes('required')) r.push('required')
	return r
})

const {
	elRef,
	errorMessage,
	showMessages,
	resetFormSubmitState: _resetFormSubmitState,
	handleAlertClose
} = useValidation(props.label, () => props.modelValue, rules, {
	error: () => props.error,
	onErrorChange: (val) => emits('update:error', val),
	onClose: () => emits('close')
})

const items = ref<map[]>([])

const composeModelValueFromItems = () => {
	const val: Record<string, any> = {}

	items.value.forEach((item) => {
		val[item.key] = item.value
	})

	const valstr = JSON.stringify(val)

	emits('update:modelValue', valstr)

	return val
}

const addItem = () => {
	items.value.push({
		key: '',
		value: ''
	})

	composeModelValueFromItems()
}

const removeItem = (_item: any, idx: number) => {
	items.value.splice(idx, 1)

	composeModelValueFromItems()
}

watchOnce(
	() => props.modelValue,
	(val) => {
		if (val == `{"":""}` || val == undefined) return

		try {
			const v: map = typeof val === 'string' ? JSON.parse(val) : val
			if (!v || typeof v !== 'object') return
			Object.keys(v).forEach((key) => {
				items.value.push({ key, value: v[key] })
			})
		} catch (e) {
			console.error('Failed to parse label', e)
		}
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
		<div class="flex flex-col gap-2 pt-1">
			<template v-for="(item, idx) in items" :key="idx">
				<div class="flex items-center gap-2">
					<UiInput
						v-model="item.key"
						placeholder="Key"
						required
						class="h-8 text-xs flex-1"
						@input="composeModelValueFromItems"
					/>
					<span class="text-muted-foreground font-bold">:</span>
					<UiInput
						v-model="item.value"
						placeholder="Value"
						required
						class="h-8 text-xs flex-1"
						@input="composeModelValueFromItems"
					/>
					<UiButton
						type="button"
						size="icon"
						variant="outline"
						class="h-8 w-8 text-destructive border-destructive/30 hover:bg-destructive/10 shrink-0"
						@click="removeItem(item, idx)"
					>
						<UiIcon name="minus" size="14px" />
					</UiButton>
				</div>
			</template>
		</div>

		<button
			type="button"
			class="mt-2 flex items-center gap-1 text-xs font-semibold text-primary hover:underline cursor-pointer"
			@click="addItem"
		>
			<UiIcon name="plus" size="14px" />
			<span>Add Label</span>
		</button>

		<AppInputError v-model="showMessages" :messages="errorMessage" @close="handleAlertClose" />
	</div>
</template>

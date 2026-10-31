<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed, provide, ref, useSlots, watch } from 'vue'
import type { z } from 'zod'
import { JsonForms } from '@jsonforms/vue'
import type { UISchemaElement } from '@jsonforms/core'
import { convertZodToJsonSchema } from '@/app/services/forms/json-schema-converter'
import { hintsToUiSchema } from '@/app/services/forms/hints-to-uischema'
import { zodFormRenderers } from './renderers'
import { AutoFormAdapter, humanizeZodIssue } from '@/app/services/forms/auto-adapter'
import UiIcon from '@/components/ui/icon/Icon.vue'
import type { UiHint } from '@@/shared/types/dynamic-form.type'

const props = withDefaults(
	defineProps<{
		schema: z.ZodTypeAny
		hints?: Record<string, UiHint>
		uischema?: UISchemaElement
		disabled?: boolean
		loading?: boolean
		initialData?: Record<string, any> | null
		transform?: (data: any) => any
	}>(),
	{
		hints: () => ({}),
		disabled: false,
		loading: false,
		initialData: null
	}
)

const emits = defineEmits<{
	submit: [validData: any]
	error: [issues: Record<string, string>]
	'update:modelValue': [value: Record<string, any>]
}>()

const slots = useSlots()
provide('dynamicZodFormSlots', slots)
provide(
	'formHints',
	computed(() => props.hints)
)

const formRef = ref<HTMLFormElement | null>(null)
const model = defineModel<Record<string, any>>({ default: () => ({}) })
const issues = ref<Record<string, string>>({})
const isSubmitting = ref(false)

provide(
	'formIssues',
	computed(() => issues.value)
)

// Convert Zod schema to JSON Schema Draft-07
const jsonSchema = computed(() => {
	return convertZodToJsonSchema(props.schema)
})

// Resolve UISchema: use custom prop if provided, or auto-generate from hints
const activeUiSchema = computed(() => {
	if (props.uischema) return props.uischema
	return hintsToUiSchema(jsonSchema.value, props.hints)
})

const initDraft = () => {
	if (props.initialData && Object.keys(props.initialData).length > 0) {
		model.value = JSON.parse(JSON.stringify(props.initialData))
	}
}

initDraft()

watch(
	() => props.initialData,
	(newVal) => {
		if (newVal && Object.keys(newVal).length > 0) {
			model.value = JSON.parse(JSON.stringify(newVal))
		}
	},
	{ deep: true }
)

// Handle data changes from JSON Forms (Ajv Real-time)
const handleJsonFormsChange = (event: { data: any; errors?: any[] }) => {
	if (event.data !== undefined) {
		model.value = event.data
		emits('update:modelValue', event.data)
	}
}

// Prepare data prior to validation (transform + deepSanitize)
const prepareDataForValidation = () => {
	let dataToValidate = JSON.parse(JSON.stringify(model.value))
	if (props.transform) {
		dataToValidate = props.transform(dataToValidate)
	}
	return AutoFormAdapter.deepSanitize(dataToValidate, props.schema, [], props.hints)
}

// Zod validation on Submit (Dual-layer Validation)
const handleSubmit = () => {
	if (isSubmitting.value) return
	isSubmitting.value = true
	try {
		const dataToValidate = prepareDataForValidation()
		const parseResult = props.schema.safeParse(dataToValidate)

		if (!parseResult.success) {
			console.error(
				'[DynamicZodForm] Validation failed:',
				parseResult.error,
				'dataToValidate was:',
				dataToValidate
			)
			const formattedIssues: Record<string, string> = {}
			for (const issue of parseResult.error.issues) {
				const pathKey = issue.path.join('.') || 'root'
				formattedIssues[pathKey] = humanizeZodIssue(issue, props.hints)
			}

			issues.value = formattedIssues
			emits('error', formattedIssues)
			return
		}

		issues.value = {}
		emits('submit', parseResult.data)
	} catch (err: any) {
		console.error('[DynamicZodForm] Error during submission:', err)
	} finally {
		setTimeout(() => {
			isSubmitting.value = false
		}, 0)
	}
}

const resetForm = () => {
	model.value = props.initialData ? JSON.parse(JSON.stringify(props.initialData)) : {}
	issues.value = {}
}

const handleCloseAlert = () => {
	issues.value = {}
}

defineExpose({
	validateAndSubmit: () => {
		if (formRef.value) {
			const submitEvent = new Event('submit', { cancelable: true, bubbles: false })
			formRef.value.dispatchEvent(submitEvent)
			if (submitEvent.defaultPrevented) {
				return
			}
		}
		handleSubmit()
	},
	validate: () => {
		const dataToValidate = prepareDataForValidation()
		return props.schema.safeParse(dataToValidate)
	},
	reset: resetForm,
	clearIssues: handleCloseAlert,
	issues,
	model
})
</script>

<template>
	<form
		ref="formRef"
		data-dynamic-zod-form="true"
		@submit.prevent="handleSubmit"
		novalidate
		class="flex flex-col gap-4"
	>
		<slot name="header" />

		<!-- Warning banner on submit validation failure -->
		<div
			v-if="Object.keys(issues).length > 0"
			data-form-error-banner="true"
			class="mb-2 form-error-banner rounded-lg border border-rose-200 bg-rose-50 dark:border-rose-900/60 dark:bg-rose-950/40 p-4 text-rose-900 dark:text-rose-200 text-sm relative"
		>
			<div class="flex items-start justify-between gap-2">
				<div class="flex items-start gap-2">
					<UiIcon name="alert-circle" class="w-5 h-5 text-rose-600 shrink-0 mt-0.5" />
					<div>
						<h5 class="font-semibold text-sm leading-tight">
							Form input contains errors. Please review the highlighted fields.
						</h5>
						<ul class="list-disc pl-5 mt-2 text-xs space-y-1">
							<li v-for="(msg, field) in issues" :key="field">
								<strong>{{ field }}:</strong> {{ msg }}
							</li>
						</ul>
					</div>
				</div>
				<button
					type="button"
					class="text-rose-500 hover:text-rose-700 dark:hover:text-rose-300"
					@click="handleCloseAlert"
				>
					<UiIcon name="close" class="w-4 h-4" />
				</button>
			</div>
		</div>

		<slot />

		<!-- JSON Forms Core Engine with BTJ UI Renderers -->
		<JsonForms
			:data="model"
			:schema="jsonSchema"
			:uischema="activeUiSchema"
			:renderers="zodFormRenderers"
			:readonly="props.disabled || props.loading"
			@change="handleJsonFormsChange"
		/>

		<slot
			name="footer"
			:submit="handleSubmit"
			:reset="resetForm"
			:isSubmitting="isSubmitting || props.loading"
		/>
	</form>
</template>

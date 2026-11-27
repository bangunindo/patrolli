<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed, provide, ref, toRaw, useSlots, watch } from 'vue'
import type { z } from 'zod'
import { useForm } from '@tanstack/vue-form'
import { introspectZodSchema } from '@/app/services/forms/zod-introspector'
import { AutoFormAdapter } from '@/app/services/forms/auto-adapter'
import {
	isDeepEqual,
	isFieldVisible,
	resolveHintForPath,
	safeClone
} from '@/app/services/forms/path-helper'
import UiIcon from '@/components/ui/icon/Icon.vue'
import DynamicFieldRenderer from './DynamicFieldRenderer.vue'
import GroupLayout from './renderers/GroupLayout.vue'
import { DYNAMIC_FORM_CONTEXT_KEY } from './useDynamicField'
import type { FieldDescriptor, UiHint } from '@@/shared/types/dynamic-form.type'

const props = withDefaults(
	defineProps<{
		schema: z.ZodType
		hints?: Record<string, UiHint>
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
}>()

const slots = useSlots()
provide('dynamicZodFormSlots', slots)

const model = defineModel<Record<string, any>>({ default: () => ({}) })
const issues = ref<Record<string, string>>({})
const isSubmitting = ref(false)
const hasSubmitted = ref(false)

const cloneData = (val: any): Record<string, any> => {
	if (val === undefined || val === null) return {}
	return safeClone(val)
}

const getInitialValues = () => {
	if (props.initialData && Object.keys(props.initialData).length > 0) {
		return cloneData(props.initialData)
	}
	if (model.value && Object.keys(model.value).length > 0) {
		return cloneData(model.value)
	}
	return {}
}

const hasInitialData = Boolean(props.initialData && Object.keys(props.initialData).length > 0)
const initialValues = getInitialValues()
if (hasInitialData) {
	model.value = cloneData(initialValues)
}

let isInternalSync = false

const form = useForm({
	defaultValues: initialValues as Record<string, any>
})

const rootValues = form.useSelector((state) => state.values)

const prepareRawData = () => {
	if (props.transform) {
		return props.transform(cloneData(model.value))
	}
	return toRaw(model.value) ?? {}
}

const prepareDataForValidation = () => {
	return AutoFormAdapter.deepSanitize(
		cloneData(prepareRawData()),
		toRaw(props.schema),
		[],
		props.hints
	)
}

const runZodValidation = () => {
	const rawData = prepareRawData()
	const result = AutoFormAdapter.toRequest(toRaw(props.schema), rawData, props.hints)
	if (!result.success) {
		return {
			success: false as const,
			issues: result.issues || {}
		}
	}
	return { success: true as const, data: result.data }
}

const syncModelFromForm = (_changedPath?: string) => {
	const cloned = cloneData(form.state.values)
	isInternalSync = true
	model.value = cloned
	isInternalSync = false

	if (hasSubmitted.value) {
		const res = runZodValidation()
		issues.value = res.success ? {} : res.issues
	}
}

watch(
	() => props.initialData,
	(newVal) => {
		const nextVal = newVal && Object.keys(newVal).length > 0 ? cloneData(newVal) : {}
		isInternalSync = true
		model.value = nextVal
		isInternalSync = false
		form.reset(cloneData(nextVal))
		issues.value = {}
		hasSubmitted.value = false
	},
	{ deep: true }
)

// Skip resetting when the echoed prop value already matches form.state.values
watch(
	() => model.value,
	(newVal) => {
		if (isInternalSync || isDeepEqual(newVal, form.state.values)) return
		form.reset(cloneData(newVal))
	},
	{ deep: true, flush: 'sync' }
)

provide(DYNAMIC_FORM_CONTEXT_KEY, {
	form,
	rootValues,
	formHints: computed(() => props.hints),
	formIssues: computed(() => issues.value),
	hasSubmitted: computed(() => hasSubmitted.value),
	formDisabled: computed(() => props.disabled || props.loading),
	syncModelFromForm
})

const descriptors = computed<FieldDescriptor[]>(() => {
	return introspectZodSchema(toRaw(props.schema), [], props.hints)
})

interface LayoutSection {
	type: 'group' | 'field'
	key: string
	order: number
	label?: string
	variant?: 'card' | 'section'
	fields?: FieldDescriptor[]
	field?: FieldDescriptor
}

const hasDynamicLayout = computed(() => {
	for (const desc of descriptors.value) {
		if (typeof desc.hints?.visibleWhen === 'function') return true
	}
	for (const [key, h] of Object.entries(props.hints || {})) {
		if (typeof h?.visibleWhen === 'function') return true
		if (key.startsWith('config.') && key.split('.').length > 2) return true
	}
	return false
})

const layoutSections = computed<LayoutSection[]>(() => {
	const groupsMap = new Map<
		string,
		{ order: number; variant?: 'card' | 'section'; fields: FieldDescriptor[] }
	>()
	const sections: LayoutSection[] = []

	// Only subscribe to rootValues when dynamic visibility or branch-specific hints exist
	const currentRoot = hasDynamicLayout.value ? rootValues.value || {} : {}

	for (const desc of descriptors.value) {
		const resolvedHint = {
			...(desc.hints || {}),
			...(resolveHintForPath(desc.pathKey, props.hints, currentRoot) || {})
		}
		if (!isFieldVisible(resolvedHint, currentRoot)) continue
		const groupName = resolvedHint.group
		const order = resolvedHint.order ?? 9999

		if (groupName) {
			if (!groupsMap.has(groupName)) {
				groupsMap.set(groupName, {
					order,
					variant: resolvedHint.groupVariant,
					fields: []
				})
			}
			const grp = groupsMap.get(groupName)!
			grp.order = Math.min(grp.order, order)
			if (resolvedHint.groupVariant && !grp.variant) {
				grp.variant = resolvedHint.groupVariant
			}
			grp.fields.push(desc)
		} else {
			sections.push({
				type: 'field',
				key: desc.pathKey,
				order,
				field: desc
			})
		}
	}

	for (const [groupName, grp] of groupsMap.entries()) {
		sections.push({
			type: 'group',
			key: `group:${groupName}`,
			order: grp.order,
			label: groupName,
			variant: grp.variant,
			fields: grp.fields
		})
	}

	sections.sort((a, b) => a.order - b.order)
	return sections
})

const handleSubmit = () => {
	hasSubmitted.value = true
	if (isSubmitting.value) return
	isSubmitting.value = true
	try {
		const result = runZodValidation()

		if (!result.success) {
			issues.value = result.issues
			emits('error', result.issues)
			return
		}

		issues.value = {}
		emits('submit', result.data)
	} catch (err: any) {
		console.error('[DynamicZodForm] Error during submission:', err)
	} finally {
		isSubmitting.value = false
	}
}

const resetForm = () => {
	hasSubmitted.value = false
	const nextVal = props.initialData ? cloneData(props.initialData) : {}
	isInternalSync = true
	model.value = nextVal
	isInternalSync = false
	form.reset(cloneData(nextVal))
	issues.value = {}
}

const handleCloseAlert = () => {
	issues.value = {}
}

defineExpose({
	validateAndSubmit: handleSubmit,
	validate: () => {
		const dataToValidate = prepareDataForValidation()
		return toRaw(props.schema).safeParse(dataToValidate)
	},
	reset: resetForm,
	clearIssues: handleCloseAlert,
	issues,
	model
})
</script>

<template>
	<form
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

		<!-- TanStack Form Recursive Field Tree -->
		<div class="flex flex-col gap-4">
			<template v-for="section in layoutSections" :key="section.key">
				<GroupLayout
					v-if="section.type === 'group' && section.fields"
					:label="section.label"
					:variant="section.variant"
					:fields="section.fields"
				/>
				<DynamicFieldRenderer
					v-else-if="section.type === 'field' && section.field"
					:descriptor="section.field"
					:path="section.field.pathKey"
				/>
			</template>
		</div>

		<slot
			name="footer"
			:submit="handleSubmit"
			:reset="resetForm"
			:isSubmitting="isSubmitting || props.loading"
		/>
	</form>
</template>

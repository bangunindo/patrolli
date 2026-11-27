// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { computed, inject, type ComputedRef, type Ref } from 'vue'
import { useField } from '@tanstack/vue-form'
import type { FieldDescriptor, UiHint } from '@@/shared/types/dynamic-form.type'
import {
	getActiveDiscriminatorType,
	getValueByPath,
	resolveHintForPath
} from '@/app/services/forms/path-helper'

export interface DynamicFormContext {
	form: any
	rootValues: Readonly<Ref<Record<string, any>>>
	formHints: ComputedRef<Record<string, UiHint>>
	formIssues: ComputedRef<Record<string, string>>
	hasSubmitted: ComputedRef<boolean>
	formDisabled: ComputedRef<boolean>
	syncModelFromForm: (changedPath?: string) => void
}

export const DYNAMIC_FORM_CONTEXT_KEY = Symbol('DynamicFormContext')
export const RESOLVED_HINT_KEY = Symbol('ResolvedFieldHint')

export function useDynamicFormContext(): DynamicFormContext {
	const ctx = inject<DynamicFormContext | undefined>(DYNAMIC_FORM_CONTEXT_KEY, undefined)
	if (!ctx) {
		throw new Error('useDynamicFormContext must be used inside DynamicZodForm')
	}
	return ctx
}

/**
 * Shared composable for all TanStack Form field renderers.
 * Binds to TanStack Form's useField, resolves UiHints, custom slots, and validation errors.
 */
export function useDynamicField(
	props: { descriptor: FieldDescriptor; path: string },
	options?: { mode?: 'value' | 'array' }
) {
	const ctx = useDynamicFormContext()
	const slots = inject<Record<string, any>>('dynamicZodFormSlots', {})

	const field = useField({
		form: ctx.form,
		name: props.path as any,
		...(options?.mode ? { mode: options.mode } : {})
	})

	const leafName = computed(() => {
		const parts = (props.path || '').split('.')
		return parts[parts.length - 1] || props.path
	})

	const injectedHint = inject<ComputedRef<UiHint> | undefined>(RESOLVED_HINT_KEY, undefined)
	const hint = computed<UiHint>(() => {
		if (injectedHint) return injectedHint.value
		const resolved = resolveHintForPath(props.path, ctx.formHints.value, ctx.rootValues.value)
		return {
			...(props.descriptor.hints || {}),
			...(resolved || {})
		}
	})

	const label = computed(() => hint.value.label || props.descriptor.label)
	const required = computed(() => props.descriptor.required)
	const disabled = computed(
		() => ctx.formDisabled.value || Boolean(hint.value.disabled) || Boolean(hint.value.readonly)
	)

	const error = computed<string>(() => {
		const issueErr = ctx.formIssues.value[props.path]
		if (issueErr) return issueErr
		const metaErr = field.state.meta.errors?.[0]
		return metaErr ? String(metaErr) : ''
	})

	// TanStack Vue Form's mode:'array' only tracks _arrayVersion, which does not change on form.reset()
	const arrayValue =
		options?.mode === 'array'
			? ctx.form.useSelector((state: any) =>
					getValueByPath(state.values, props.path.split('.'))
				)
			: null

	const value = computed(() => {
		if (arrayValue) {
			// Retain reactive dependency on field.state.value so _arrayVersion mutations trigger updates
			void field.state.value
			return arrayValue.value
		}
		return field.state.value
	})

	const setValue = (val: any) => {
		field.api.handleChange(val)
		ctx.syncModelFromForm(props.path)
	}

	const customSlot = computed(() => {
		const pathKey = props.path
		const activeType = getActiveDiscriminatorType(ctx.rootValues.value)

		if (activeType && pathKey) {
			const parts = pathKey.split('.')
			if (parts[0] === 'config' && parts.length > 1) {
				const branchSlotKey = ['config', activeType, ...parts.slice(1)].join('.')
				if (slots[`field:${branchSlotKey}`]) {
					return slots[`field:${branchSlotKey}`]
				}
			}
		}

		return slots[`field:${pathKey}`] || null
	})

	const slotProps = computed(() => ({
		value: value.value,
		modelValue: value.value,
		'model-value': value.value,
		error: error.value,
		updateModelValue: setValue,
		'update-model-value': setValue,
		handleChange: setValue,
		'handle-change': setValue
	}))

	return {
		ctx,
		fieldApi: field.api,
		field,
		value,
		setValue,
		error,
		hint,
		label,
		required,
		disabled,
		leafName,
		customSlot,
		slotProps
	}
}

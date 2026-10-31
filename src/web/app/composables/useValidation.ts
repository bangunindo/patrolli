// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import {
	computed,
	getCurrentInstance,
	onMounted,
	onUnmounted,
	ref,
	toValue,
	watch,
	type ComputedRef,
	type MaybeRefOrGetter,
	type Ref,
	type WritableComputedRef
} from 'vue'
import type { z } from 'zod'
import { validateRules, type ValidationContext } from '@/app/services/forms/format-registry'
import { humanizeZodIssue } from '@/app/services/forms/auto-adapter'

export interface FieldValidationOptions {
	label?: string
	value?: MaybeRefOrGetter<any>
	rules?: MaybeRefOrGetter<string[]>
	error?: MaybeRefOrGetter<string | string[] | null | undefined>
	schema?: z.ZodTypeAny
	path?: string | string[]
	disabled?: MaybeRefOrGetter<boolean>
	onClose?: () => void
	onErrorChange?: (err: string) => void
}

export interface ValidationReturn {
	elRef: Ref<HTMLElement | null>
	messages: Ref<string[]>
	errorMessage: ComputedRef<string>
	showMessages: WritableComputedRef<boolean>
	isValid: ComputedRef<boolean>
	hasError: ComputedRef<boolean>
	validate: () => string[]
	resetFormSubmitState: () => void
	clearError: () => void
	setError: (msg: string | string[]) => void
	handleAlertClose: () => void
}

/**
 * Centralized Validation Engine Composable
 * Integrates Format Validator Registry, Zod Schema validation, external error props,
 * and form submission lifecycle centrally for atomic inputs and dynamic forms.
 */
export function useFieldValidation(
	labelOrOptions: string | FieldValidationOptions,
	value?: MaybeRefOrGetter<any>,
	rules?: MaybeRefOrGetter<string[]>,
	options?: FieldValidationOptions
): ValidationReturn {
	// Normalize input parameters
	const isOptionsObject = typeof labelOrOptions === 'object' && labelOrOptions !== null
	const resolvedOptions: FieldValidationOptions = isOptionsObject
		? (labelOrOptions as FieldValidationOptions)
		: { ...(options || {}), label: labelOrOptions as string }

	const targetValue: MaybeRefOrGetter<any> = isOptionsObject ? resolvedOptions.value : value
	const targetRules: MaybeRefOrGetter<string[]> = isOptionsObject
		? resolvedOptions.rules || []
		: rules || []
	const fieldLabel = resolvedOptions.label || 'Field'

	const elRef = ref<HTMLElement | null>(null)
	const isOnSubmit = ref(false)
	const isDismissed = ref(false)
	const messages = ref<string[]>([])

	const externalError = computed<string[]>(() => {
		const raw = toValue(resolvedOptions.error)
		if (!raw) return []
		if (Array.isArray(raw)) {
			return raw.filter((item): item is string =>
				Boolean(item && typeof item === 'string' && item.trim().length > 0)
			)
		}
		if (typeof raw === 'string' && raw.trim().length > 0) {
			return [raw.trim()]
		}
		return []
	})

	const allMessages = computed<string[]>(() => {
		if (isDismissed.value) return []
		const list = [...messages.value]
		for (const err of externalError.value) {
			if (!list.includes(err)) {
				list.push(err)
			}
		}
		return list
	})

	const errorMessage = computed<string>(() => allMessages.value.join(' | '))

	const hasError = computed<boolean>(() => allMessages.value.length > 0)

	const showMessages = computed<boolean>({
		get: () =>
			!isDismissed.value && (messages.value.length > 0 || externalError.value.length > 0),
		set: (val: boolean) => {
			if (!val) {
				handleAlertClose()
			} else {
				isDismissed.value = false
			}
		}
	})

	const validate = (): string[] => {
		const resolvedVal = toValue(targetValue)
		const activeRules = toValue(targetRules) || []
		const errs: string[] = []

		// 1. Validate Zod Schema if provided in options
		if (resolvedOptions.schema) {
			const parsed = resolvedOptions.schema.safeParse(resolvedVal)
			if (!parsed.success) {
				for (const issue of parsed.error.issues) {
					errs.push(humanizeZodIssue(issue))
				}
			}
		}

		// 2. Validate Format Registry Rules
		if (activeRules.length > 0) {
			const context: ValidationContext = {
				path: [fieldLabel],
				pathKey: fieldLabel,
				label: fieldLabel
			}
			const ruleErrs = validateRules(resolvedVal, activeRules, context)
			for (const rErr of ruleErrs) {
				if (!errs.includes(rErr)) {
					errs.push(rErr)
				}
			}
		}

		return errs
	}

	const updateInvalidState = () => {
		if (hasError.value) {
			elRef.value?.setAttribute('data-invalid', 'true')
			elRef.value?.classList.add('has-error')
		} else {
			elRef.value?.removeAttribute('data-invalid')
			elRef.value?.classList.remove('has-error')
		}
	}

	const runValidation = () => {
		isDismissed.value = false
		messages.value = validate()
		updateInvalidState()
	}

	const handleAlertClose = () => {
		isDismissed.value = true
		messages.value = []
		isOnSubmit.value = false
		updateInvalidState()
		resolvedOptions.onErrorChange?.('')
		resolvedOptions.onClose?.()
	}

	const clearError = () => {
		isDismissed.value = false
		messages.value = []
		updateInvalidState()
		resolvedOptions.onErrorChange?.('')
	}

	const setError = (msg: string | string[]) => {
		isDismissed.value = false
		if (Array.isArray(msg)) {
			messages.value = msg.filter(Boolean)
		} else if (msg) {
			messages.value = [msg]
		} else {
			messages.value = []
		}
		updateInvalidState()
	}

	const resetFormSubmitState = () => {
		isOnSubmit.value = false
		isDismissed.value = true
		messages.value = []
		updateInvalidState()
		resolvedOptions.onErrorChange?.('')
	}

	// Watch for input value changes
	watch(
		() => toValue(targetValue),
		() => {
			isDismissed.value = false
			if (externalError.value.length > 0) {
				resolvedOptions.onErrorChange?.('')
			}
			if (isOnSubmit.value) {
				runValidation()
			} else {
				updateInvalidState()
			}
		},
		{ deep: true }
	)

	// Watch for rule changes
	watch(
		() => toValue(targetRules),
		() => {
			if (isOnSubmit.value) {
				runValidation()
			}
		},
		{ deep: true }
	)

	// Watch external error prop
	watch(
		() => externalError.value,
		(newVal) => {
			if (newVal.length > 0) {
				isDismissed.value = false
			}
			updateInvalidState()
		},
		{ deep: true }
	)

	// Watch hasError status
	watch(
		() => hasError.value,
		() => {
			updateInvalidState()
		}
	)

	const onFormSubmit = (event: Event) => {
		const activeForm = cachedForm || (elRef.value?.closest('form') as HTMLFormElement | null)

		isOnSubmit.value = true
		runValidation()

		if (hasError.value) {
			// Prevent form submission
			event.preventDefault()

			// Scroll and focus on the first invalid field
			setTimeout(() => {
				const form = activeForm || elRef.value?.closest('form')
				const firstInvalidField = form?.querySelector('[data-invalid="true"]')

				if (firstInvalidField === elRef.value) {
					elRef.value?.scrollIntoView({
						behavior: 'smooth',
						block: 'center'
					})
					const focusable = elRef.value?.matches('input, select, textarea')
						? elRef.value
						: elRef.value?.querySelector('input, select, textarea')
					;(focusable as HTMLElement)?.focus()
				}
			}, 0)
		}
	}

	let cachedForm: HTMLFormElement | null = null

	if (getCurrentInstance()) {
		onMounted(() => {
			cachedForm = elRef.value?.closest('form') as HTMLFormElement | null
			if (cachedForm) {
				cachedForm.addEventListener('submit', onFormSubmit, true)
			}
			updateInvalidState()
		})

		onUnmounted(() => {
			if (cachedForm) {
				cachedForm.removeEventListener('submit', onFormSubmit, true)
			}
		})
	}

	return {
		elRef,
		messages,
		errorMessage,
		showMessages,
		isValid: computed(() => !hasError.value),
		hasError,
		validate: () => {
			isOnSubmit.value = true
			isDismissed.value = false
			runValidation()
			return allMessages.value
		},
		resetFormSubmitState,
		clearError,
		setError,
		handleAlertClose
	}
}

// Export alias for backward compatibility
export const useValidation = useFieldValidation

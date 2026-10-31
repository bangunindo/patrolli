<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { useValidation } from '@/composables/useValidation'
import AppInputError from '@/components/app/input/AppInputError.vue'
import UiIcon from '@/components/ui/icon/Icon.vue'
import UiCheckbox from '@/components/ui/checkbox/Checkbox.vue'
import { cn } from '@/app/lib/utils'
import type { CSSProperties } from 'vue'
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'

interface SelectProps {
	modelValue: any
	options: any
	label?: string
	labelClass?: string
	labelKey?: string
	valueKey?: string
	multiple?: boolean
	placeholder?: string
	variant?: 'default' | 'outline'
	color?: 'primary' | 'success' | 'danger' | 'warning' | 'neutral'
	required?: boolean
	helper?: string
	helperClass?: string
	disabled?: boolean
	clearable?: boolean
	error?: string | string[]
	rules?: string[]
}

const props = withDefaults(defineProps<SelectProps>(), {
	label: '',
	labelClass: '',
	options: () => [],
	labelKey: 'label',
	valueKey: 'value',
	multiple: false,
	placeholder: 'NULL',
	variant: 'outline',
	color: 'primary',
	required: false,
	helper: '',
	helperClass: '',
	disabled: false,
	clearable: false,
	error: ''
})

const emit = defineEmits<{
	(e: 'update:modelValue', value: any): void
	(e: 'update:error', value: string): void
	(e: 'close'): void
}>()

const isOpen = ref(false)
const isInvalid = ref(false)
const selectRef = ref<HTMLElement | null>(null)
const triggerRef = ref<HTMLElement | null>(null)
const dropdownRef = ref<HTMLElement | null>(null)

// 1. Initial setup with visibility: hidden to prevent flickering or misplacement
const dropdownStyle = ref<CSSProperties>({
	position: 'absolute',
	top: '0px',
	left: '0px',
	width: 'auto',
	visibility: 'hidden'
})

const safeOptions = computed(() => props.options || [])

const updateDropdownPosition = () => {
	if (!triggerRef.value || !dropdownRef.value) return

	const rect = triggerRef.value.getBoundingClientRect()
	const margin = 4

	// Measure natural element height without word-wrap interference now that width is defined
	const dropdownHeight = dropdownRef.value.offsetHeight
	const spaceBelow = window.innerHeight - rect.bottom
	const spaceAbove = rect.top

	let topPosition = rect.bottom + window.scrollY + margin

	// If bounded at the bottom, flip directly above the trigger without guessing the height
	if (spaceBelow < dropdownHeight && spaceAbove > spaceBelow) {
		topPosition = rect.top + window.scrollY - dropdownHeight - margin
	}

	dropdownStyle.value = {
		position: 'absolute',
		top: `${topPosition}px`,
		left: `${rect.left + window.scrollX}px`,
		width: `${rect.width}px`,
		visibility: 'visible' // Display once position calculation is 100% accurate
	}
}

const toggleDropdown = () => {
	if (props.disabled) return
	isOpen.value = !isOpen.value
	if (isOpen.value) {
		isInvalid.value = false
		// 2. Set width & horizontal position immediately on click to prevent layout jumps upon DOM entry
		if (triggerRef.value) {
			const rect = triggerRef.value.getBoundingClientRect()
			dropdownStyle.value = {
				position: 'absolute',
				left: `${rect.left + window.scrollX}px`,
				width: `${rect.width}px`,
				top: '0px',
				visibility: 'hidden'
			}
		}
	}
}

watch(isOpen, async (value) => {
	if (value) {
		await nextTick()
		updateDropdownPosition()
		window.addEventListener('scroll', updateDropdownPosition, true)
		window.addEventListener('resize', updateDropdownPosition)
	} else {
		window.removeEventListener('scroll', updateDropdownPosition, true)
		window.removeEventListener('resize', updateDropdownPosition)
		dropdownStyle.value.visibility = 'hidden' // Hide again when closed
	}
})

// 3. Ensure recalculation runs right as dropdown element mounts in the DOM
watch(dropdownRef, async (newEl) => {
	if (newEl) {
		await nextTick()
		updateDropdownPosition()
	}
})

watch(
	() => props.modelValue,
	(val) => {
		if (hasValue.value) {
			isInvalid.value = false
		}
		selected.value = val
	},
	{ deep: true }
)

const handleInvalid = (event: Event) => {
	event.preventDefault()
	isInvalid.value = true
}

function getOptionValue(option: any) {
	if (option === null || option === undefined) return option
	if (typeof option === 'object' && props.valueKey in option) {
		return option[props.valueKey]
	}
	return option
}

const isSelected = (option: any) => {
	if (option === null || option === undefined) return false
	const optionValue = getOptionValue(option)
	if (props.multiple) {
		return Array.isArray(props.modelValue) && props.modelValue.includes(optionValue)
	}
	return props.modelValue === optionValue
}

const selectOption = (option: any) => {
	if (option === null || option === undefined) return
	const optionValue = getOptionValue(option)

	if (props.multiple) {
		const currentValues = Array.isArray(props.modelValue) ? [...props.modelValue] : []
		const index = currentValues.indexOf(optionValue)
		if (index > -1) {
			currentValues.splice(index, 1)
		} else {
			currentValues.push(optionValue)
		}
		selected.value = currentValues
		emit('update:modelValue', currentValues)
		nextTick(updateDropdownPosition)
	} else {
		selected.value = optionValue
		emit('update:modelValue', optionValue)
		isOpen.value = false
	}
}

const clearSelection = (event: MouseEvent) => {
	event.stopPropagation()
	const emptyValue = props.multiple ? [] : null
	selected.value = emptyValue
	emit('update:modelValue', emptyValue)
}

const hasValue = computed(() => {
	if (props.multiple) {
		return Array.isArray(props.modelValue) && props.modelValue.length > 0
	}
	return props.modelValue !== null && props.modelValue !== undefined && props.modelValue !== ''
})

const displayText = computed(() => {
	if (props.multiple) {
		if (!Array.isArray(props.modelValue) || props.modelValue.length === 0) {
			return props.placeholder
		}
		return safeOptions.value
			.filter((opt) => opt && (props.modelValue as any[]).includes(opt[props.valueKey]))
			.map((opt) => opt[props.labelKey])
			.join(', ')
	} else {
		const selectedOpt = safeOptions.value.find(
			(opt) => opt && opt[props.valueKey] === props.modelValue
		)

		return selectedOpt ? selectedOpt[props.labelKey] : props.placeholder
	}
})

const handleClickOutside = (event: MouseEvent) => {
	const target = event.target as HTMLElement
	if (target.closest('.select-dropdown-teleported')) return
	if (triggerRef.value && !triggerRef.value.contains(target)) {
		isOpen.value = false
	}
}

onMounted(() => {
	selected.value = props.modelValue
	document.addEventListener('click', handleClickOutside)
})

onUnmounted(() => {
	isOpen.value = false
	document.removeEventListener('click', handleClickOutside)
	window.removeEventListener('scroll', updateDropdownPosition, true)
	window.removeEventListener('resize', updateDropdownPosition)
})

const selected = ref()
const rules = computed(() => {
	const r: string[] = [...(props.rules || [])]
	if (props.multiple) {
		if (props.required && !r.includes('array')) r.push('array')
	} else {
		if (props.required && !r.includes('required')) r.push('required')
	}
	return r
})

const {
	elRef,
	errorMessage,
	showMessages,
	hasError,
	resetFormSubmitState: _resetFormSubmitState,
	handleAlertClose
} = useValidation(props.label, selected, rules, {
	error: () => props.error,
	onErrorChange: (val) => emit('update:error', val),
	onClose: () => emit('close')
})

function getOptionLabel(option: any) {
	if (option === null || option === undefined) return ''
	if (typeof option === 'object' && props.labelKey in option) {
		return option[props.labelKey]
	}
	return String(option)
}
</script>

<template>
	<div ref="elRef" class="w-full space-y-1.5">
		<label
			v-if="label"
			:class="cn('text-sm font-semibold text-foreground flex items-center gap-1', labelClass)"
		>
			<span>{{ label }}</span>
			<span v-if="required" class="text-destructive">*</span>
		</label>

		<div ref="selectRef" class="relative w-full">
			<div
				ref="triggerRef"
				:class="
					cn(
						'select-trigger flex h-9 w-full items-center justify-between rounded-md border border-input bg-transparent dark:bg-slate-900/40 px-3 py-1 text-sm shadow-sm transition-colors select-none cursor-pointer hover:border-slate-400 dark:hover:border-slate-600 focus:outline-none focus:ring-1 focus:ring-ring focus:border-input',
						isOpen ? 'ring-1 ring-ring border-input' : '',
						isInvalid ||
							Boolean(props.error) ||
							hasError ||
							(errorMessage && showMessages)
							? 'border-destructive focus:ring-destructive ring-destructive'
							: '',
						disabled ? 'cursor-not-allowed opacity-50' : ''
					)
				"
				tabindex="0"
				@click="toggleDropdown"
				@keydown.enter.prevent="toggleDropdown"
				@keydown.space.prevent="toggleDropdown"
				@keydown.down.prevent="isOpen = true"
			>
				<span
					class="truncate leading-none"
					:class="hasValue ? 'text-foreground' : 'text-muted-foreground'"
				>
					{{ displayText }}
				</span>
				<div class="flex items-center gap-1.5 ml-2 shrink-0">
					<span
						v-if="clearable && hasValue && !disabled"
						class="text-muted-foreground hover:text-destructive cursor-pointer transition-colors p-0.5"
						title="Clear"
						@click.stop="clearSelection"
					>
						<UiIcon name="close" size="14px" />
					</span>
					<span
						class="text-muted-foreground transition-transform duration-200"
						:class="{ 'rotate-180': isOpen }"
					>
						<UiIcon name="chevron-down" size="16px" />
					</span>
				</div>
			</div>

			<teleport to="body">
				<transition name="fade">
					<div
						v-if="isOpen"
						ref="dropdownRef"
						class="select-dropdown-teleported bg-popover text-popover-foreground border border-border rounded-md shadow-md"
						data-dismissable-layer=""
						data-reka-portal=""
						:style="dropdownStyle"
						@pointerdown.stop
						@mousedown.stop
					>
						<div
							v-for="option in safeOptions"
							:key="String(getOptionValue(option) ?? option)"
							class="select-option flex items-center justify-between px-3 py-1.5 text-sm cursor-pointer transition-colors hover:bg-accent hover:text-accent-foreground select-none"
							:class="{
								'bg-accent/60 font-medium text-foreground': isSelected(option)
							}"
							@pointerdown.stop="selectOption(option)"
							@mousedown.stop="selectOption(option)"
							@click.stop="selectOption(option)"
						>
							<div class="flex items-center gap-2 truncate">
								<UiCheckbox
									v-if="multiple"
									:checked="isSelected(option)"
									class="pointer-events-none"
								/>
								<span class="truncate">{{ getOptionLabel(option) }}</span>
							</div>
							<UiIcon
								v-if="!multiple && isSelected(option)"
								name="check"
								size="14px"
								class="text-primary shrink-0 ml-2"
							/>
						</div>
						<div
							v-if="safeOptions.length === 0"
							class="p-3 text-sm text-center text-muted-foreground"
						>
							Tidak ada data.
						</div>
					</div>
				</transition>
			</teleport>

			<span
				v-if="helper"
				:class="cn('text-xs text-muted-foreground block mt-1', helperClass)"
			>
				{{ helper }}
			</span>

			<input
				type="text"
				class="hidden-validation-input"
				:required="required"
				:value="hasValue ? 'filled' : ''"
				tabindex="-1"
				aria-hidden="true"
				@invalid="handleInvalid"
			/>
		</div>

		<AppInputError v-model="showMessages" :messages="errorMessage" @close="handleAlertClose" />
	</div>
</template>

<style scoped>
.hidden-validation-input {
	position: absolute;
	bottom: 0;
	left: 50%;
	transform: translateX(-50%);
	width: 1px;
	height: 1px;
	opacity: 0;
	pointer-events: none;
	z-index: -1;
	border: none;
}
</style>

<style>
.select-dropdown-teleported {
	pointer-events: auto !important;
	z-index: 9999;
	max-height: 240px;
	overflow-y: auto;
	box-sizing: border-box;
	padding: 4px;
}
.fade-enter-active,
.fade-leave-active {
	transition:
		opacity 0.15s ease,
		transform 0.15s ease;
}
.fade-enter-from,
.fade-leave-to {
	opacity: 0;
	transform: translateY(-4px);
}
</style>

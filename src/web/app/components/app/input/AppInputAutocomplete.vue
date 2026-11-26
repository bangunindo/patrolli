<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed, onBeforeUnmount, reactive, ref, useId, watch } from 'vue'
import { useValidation } from '@/composables/useValidation'
import { cn } from '@/app/lib/utils'
import AppInputError from '@/components/app/input/AppInputError.vue'
import UiIcon from '@/components/ui/icon/Icon.vue'
import UiBadge from '@/components/ui/badge/Badge.vue'
import {
	Tooltip as UiTooltip,
	TooltipContent as UiTooltipContent,
	TooltipProvider as UiTooltipProvider,
	TooltipTrigger as UiTooltipTrigger
} from '@/components/ui/tooltip'
import {
	Combobox as UiCombobox,
	ComboboxAnchor as UiComboboxAnchor,
	ComboboxEmpty as UiComboboxEmpty,
	ComboboxGroup as UiComboboxGroup,
	ComboboxInput as UiComboboxInput,
	ComboboxItem as UiComboboxItem,
	ComboboxList as UiComboboxList,
	ComboboxTrigger as UiComboboxTrigger
} from '@/components/ui/combobox'

export interface AutocompleteProps {
	options?: any[]
	label?: string
	labelClass?: string
	labelKey?: string
	subLabelKey?: string
	valueKey?: string
	placeholder?: string
	emptyText?: string
	helper?: string
	helperClass?: string
	multiple?: boolean
	maxVisibleTags?: number
	allowCustomValue?: boolean
	remote?: boolean
	loading?: boolean
	debounceMs?: number
	clearable?: boolean
	required?: boolean
	disabled?: boolean
	readonly?: boolean
	preIcon?: string
	portal?: boolean
	error?: string | string[]
	rules?: string[]
}

interface NormalizedOption {
	raw: any
	label: string
	subLabel?: string
	value: any
	disabled: boolean
}

const props = withDefaults(defineProps<AutocompleteProps>(), {
	options: () => [],
	label: '',
	labelClass: '',
	labelKey: 'label',
	subLabelKey: '',
	valueKey: 'value',
	placeholder: 'Search or select...',
	emptyText: 'No results found.',
	helper: '',
	helperClass: '',
	multiple: false,
	maxVisibleTags: 2,
	allowCustomValue: false,
	remote: false,
	loading: false,
	debounceMs: 300,
	clearable: false,
	required: false,
	disabled: false,
	readonly: false,
	preIcon: '',
	portal: true,
	error: ''
})

const emits = defineEmits<{
	search: [query: string]
	select: [option: any]
	clear: []
	close: []
	'update:error': [value: string]
}>()

const model = defineModel<any>(undefined)
const inputId = useId()

const isOpen = ref(false)
const searchTerm = ref('')
// Preserves display labels across remote option reloads
const labelCache = reactive(new Map<any, string>())
let searchTimer: ReturnType<typeof setTimeout> | null = null
let cancelNextClose = false

const isEmpty = (val: unknown): boolean => val === null || val === undefined || val === ''

const isInteractive = computed(() => !props.disabled && !props.readonly)

const normalizedOptions = computed<NormalizedOption[]>(() => {
	return (props.options || []).map((opt) => {
		if (opt !== null && typeof opt === 'object') {
			const value = opt[props.valueKey] ?? opt[props.labelKey]
			const label = String(opt[props.labelKey] ?? opt[props.valueKey] ?? '')
			const rawSubLabel = props.subLabelKey ? opt[props.subLabelKey] : undefined
			const subLabel = !isEmpty(rawSubLabel) ? String(rawSubLabel) : undefined
			return {
				raw: opt,
				label,
				subLabel,
				value,
				disabled: Boolean(opt.disabled)
			}
		}
		return {
			raw: opt,
			label: String(opt),
			value: opt,
			disabled: false
		}
	})
})

const selectedValues = computed<any[]>(() => {
	if (props.multiple) {
		return Array.isArray(model.value) ? model.value : []
	}
	return isEmpty(model.value) ? [] : [model.value]
})

const hasValue = computed(() => selectedValues.value.length > 0)
const trimmedQuery = computed(() => searchTerm.value.trim())

const isSelected = (value: any): boolean => selectedValues.value.includes(value)

const getLabelForValue = (value: any): string => {
	if (isEmpty(value)) return ''
	return labelCache.get(value) ?? String(value)
}

const tagDisplay = computed(() => {
	const limit = props.maxVisibleTags && props.maxVisibleTags > 0 ? props.maxVisibleTags : 0
	const visible = limit ? selectedValues.value.slice(0, limit) : selectedValues.value
	const hidden = limit ? selectedValues.value.slice(limit) : []
	return {
		visible,
		hiddenCount: hidden.length,
		hiddenLabels: hidden.map(getLabelForValue)
	}
})

const filteredOptions = computed<NormalizedOption[]>(() => {
	if (props.remote) return normalizedOptions.value
	const query = trimmedQuery.value.toLowerCase()
	if (!query) return normalizedOptions.value

	// In single mode, show full list when input still matches the currently selected label
	if (
		!props.multiple &&
		hasValue.value &&
		query === getLabelForValue(model.value).toLowerCase()
	) {
		return normalizedOptions.value
	}

	return normalizedOptions.value.filter(
		(opt) =>
			opt.label.toLowerCase().includes(query) ||
			String(opt.value).toLowerCase().includes(query) ||
			Boolean(opt.subLabel?.toLowerCase().includes(query))
	)
})

const canCreateCustom = computed(() => {
	if (!props.allowCustomValue || !trimmedQuery.value) return false
	if (props.multiple && isSelected(trimmedQuery.value)) return false
	const q = trimmedQuery.value.toLowerCase()
	return !normalizedOptions.value.some(
		(opt) => opt.label.toLowerCase() === q || String(opt.value).toLowerCase() === q
	)
})

const syncSearchFromModel = () => {
	if (props.multiple) return
	searchTerm.value = hasValue.value ? getLabelForValue(model.value) : ''
}

watch(
	normalizedOptions,
	(opts) => {
		for (const opt of opts) {
			labelCache.set(opt.value, opt.label)
		}
		if (!isOpen.value) {
			syncSearchFromModel()
		}
	},
	{ immediate: true }
)

watch(
	() => model.value,
	() => {
		if (!isOpen.value) {
			syncSearchFromModel()
		}
	},
	{ immediate: true }
)

const commitCustomValueOnClose = (trimmed: string) => {
	if (!trimmed) {
		if (hasValue.value) {
			model.value = ''
			resetFormSubmitState()
		}
		return
	}
	if (trimmed === getLabelForValue(model.value)) return
	const matched = normalizedOptions.value.find(
		(opt) => opt.label.toLowerCase() === trimmed.toLowerCase()
	)
	model.value = matched ? matched.value : trimmed
	searchTerm.value = matched ? matched.label : trimmed
	resetFormSubmitState()
}

watch(isOpen, (open) => {
	const trimmed = trimmedQuery.value
	if (open) {
		cancelNextClose = false
		const isFreshOpen = props.multiple
			? !trimmed
			: !trimmed || trimmed === getLabelForValue(model.value)
		if (!searchTimer && isFreshOpen) {
			resetRemoteSearch()
		}
		return
	}

	const isCancelled = cancelNextClose
	cancelNextClose = false
	clearSearchTimer()

	if (props.multiple) {
		searchTerm.value = ''
		return
	}

	if (props.allowCustomValue && !isCancelled) {
		commitCustomValueOnClose(trimmed)
	} else {
		syncSearchFromModel()
	}
})

let lastEmittedSearch = ''

const clearSearchTimer = () => {
	if (searchTimer) {
		clearTimeout(searchTimer)
		searchTimer = null
	}
}

const emitSearchNow = (query: string) => {
	clearSearchTimer()
	lastEmittedSearch = query
	emits('search', query)
}

const resetRemoteSearch = () => {
	if (props.remote && lastEmittedSearch !== '') {
		emitSearchNow('')
		return
	}
	clearSearchTimer()
}

const scheduleSearchEmit = (query: string) => {
	clearSearchTimer()
	if (props.debounceMs <= 0) {
		emitSearchNow(query)
		return
	}
	searchTimer = setTimeout(() => emitSearchNow(query), props.debounceMs)
}

onBeforeUnmount(clearSearchTimer)

const handleSearchInput = (event: Event) => {
	const target = event.target as HTMLInputElement
	searchTerm.value = target.value
	if (!isOpen.value && isInteractive.value) {
		isOpen.value = true
	}
	scheduleSearchEmit(target.value)
}

const commitSelection = (value: any, label: string, raw: any, allowToggle: boolean) => {
	labelCache.set(value, label)

	if (props.multiple) {
		const idx = selectedValues.value.indexOf(value)
		if (idx >= 0) {
			if (allowToggle) {
				model.value = selectedValues.value.filter((_, i) => i !== idx)
			}
		} else {
			model.value = [...selectedValues.value, value]
		}
		searchTerm.value = ''
	} else {
		model.value = value
		searchTerm.value = label
		isOpen.value = false
	}

	emits('select', raw)
	resetFormSubmitState()
}

const handleSelectOption = (opt: NormalizedOption) => {
	if (!isInteractive.value || opt.disabled) return
	commitSelection(opt.value, opt.label, opt.raw, true)
}

const handleSelectCustom = () => {
	const customVal = trimmedQuery.value
	if (!isInteractive.value || !customVal) return
	commitSelection(customVal, customVal, customVal, false)
}

const removeSelectedValue = (value: any) => {
	if (!isInteractive.value || !props.multiple) return
	model.value = selectedValues.value.filter((v) => v !== value)
	resetFormSubmitState()
}

const handleClear = () => {
	if (!isInteractive.value) return
	model.value = props.multiple ? [] : ''
	searchTerm.value = ''
	if (isOpen.value) {
		resetRemoteSearch()
	}
	emits('clear')
	resetFormSubmitState()
}

const handleKeydown = (event: KeyboardEvent) => {
	if (!isInteractive.value) return

	if (event.key === 'Escape') {
		if (isOpen.value) {
			event.stopPropagation()
			cancelNextClose = true
			isOpen.value = false
		}
		return
	}

	// Prevent parent DropdownMenu typeahead from stealing focus while typing
	event.stopPropagation()

	if (event.key === 'Backspace' && props.multiple && !searchTerm.value && hasValue.value) {
		const lastVal = selectedValues.value[selectedValues.value.length - 1]
		removeSelectedValue(lastVal)
		return
	}

	if (event.key === 'Enter' && canCreateCustom.value && filteredOptions.value.length === 0) {
		event.preventDefault()
		handleSelectCustom()
	}
}

const handleOpenChange = (open: boolean) => {
	if (!isInteractive.value && open) return
	isOpen.value = open
}

const rules = computed(() => {
	const r: string[] = [...(props.rules || [])]
	if (props.required && !r.includes('required')) r.push('required')
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
			:for="inputId"
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

		<!-- Omit :model-value so reka-ui ListboxRoot does not scroll back to the first checked item on selection -->
		<UiCombobox
			:open="isOpen"
			:multiple="props.multiple"
			:disabled="props.disabled"
			:ignore-filter="true"
			:open-on-focus="isInteractive"
			:reset-search-term-on-blur="false"
			@update:open="handleOpenChange"
		>
			<UiComboboxAnchor>
				<div
					:class="
						cn(
							'flex min-h-9 w-full flex-wrap items-center gap-1.5 rounded-md border border-input bg-background px-3 py-1 text-sm shadow-sm transition-colors focus-within:ring-1 focus-within:ring-ring',
							props.disabled && 'cursor-not-allowed opacity-50',
							props.readonly && 'bg-muted/40',
							errorMessage && showMessages
								? 'border-destructive focus-within:ring-destructive'
								: ''
						)
					"
				>
					<UiIcon
						v-if="props.preIcon"
						:name="props.preIcon"
						size="16px"
						class="text-muted-foreground shrink-0"
					/>

					<template v-if="props.multiple && hasValue">
						<UiBadge
							v-for="val in tagDisplay.visible"
							:key="String(val)"
							variant="secondary"
							class="gap-1 pr-1 font-normal max-w-full"
						>
							<span class="truncate">{{ getLabelForValue(val) }}</span>
							<button
								v-if="isInteractive"
								type="button"
								class="ml-0.5 rounded-sm p-0.5 hover:bg-muted-foreground/20 focus:outline-none"
								:aria-label="`Remove ${getLabelForValue(val)}`"
								@click.stop="removeSelectedValue(val)"
							>
								<UiIcon name="close" size="12px" />
							</button>
						</UiBadge>

						<UiTooltipProvider v-if="tagDisplay.hiddenCount > 0" :delay-duration="200">
							<UiTooltip>
								<UiTooltipTrigger as-child>
									<UiBadge
										variant="outline"
										class="font-normal text-muted-foreground cursor-default"
									>
										+{{ tagDisplay.hiddenCount }} more
									</UiBadge>
								</UiTooltipTrigger>
								<UiTooltipContent side="top" class="max-w-64 pr-1.5">
									<ul
										class="autocomplete-tooltip-scroll max-h-60 overflow-y-auto overscroll-contain space-y-0.5 pr-1.5 text-xs"
									>
										<li
											v-for="(itemLabel, idx) in tagDisplay.hiddenLabels"
											:key="idx"
											class="truncate"
										>
											{{ itemLabel }}
										</li>
									</ul>
								</UiTooltipContent>
							</UiTooltip>
						</UiTooltipProvider>
					</template>

					<UiComboboxInput
						:id="inputId"
						:model-value="searchTerm"
						:display-value="() => searchTerm"
						:placeholder="props.multiple && hasValue ? '' : props.placeholder"
						:disabled="props.disabled"
						:readonly="props.readonly"
						class="h-7 min-w-[80px] flex-1 border-0 p-0 shadow-none focus-visible:ring-0"
						@input="handleSearchInput"
						@keydown="handleKeydown"
					/>

					<div class="ml-auto flex items-center gap-1 shrink-0">
						<button
							v-if="props.clearable && hasValue && isInteractive"
							type="button"
							aria-label="Clear selection"
							class="rounded-sm p-0.5 text-muted-foreground hover:text-foreground focus:outline-none"
							@click.stop="handleClear"
						>
							<UiIcon name="close" size="14px" />
						</button>

						<UiComboboxTrigger
							:disabled="!isInteractive"
							class="flex items-center text-muted-foreground hover:text-foreground disabled:pointer-events-none"
						>
							<UiIcon
								name="chevron-down"
								size="16px"
								:class="
									cn('transition-transform duration-150', isOpen && 'rotate-180')
								"
							/>
						</UiComboboxTrigger>
					</div>
				</div>
			</UiComboboxAnchor>

			<UiComboboxList :portal="props.portal">
				<div
					v-if="props.loading"
					class="flex items-center justify-center gap-2 py-6 text-sm text-muted-foreground"
					role="status"
				>
					<UiIcon name="loading" size="16px" class="animate-spin" />
					<span>Loading...</span>
				</div>

				<template v-else>
					<UiComboboxGroup v-if="filteredOptions.length > 0 || canCreateCustom">
						<UiComboboxItem
							v-for="opt in filteredOptions"
							:key="String(opt.value)"
							:value="opt.value"
							:disabled="opt.disabled"
							@select.prevent="handleSelectOption(opt)"
						>
							<slot
								name="option"
								:option="opt.raw"
								:label="opt.label"
								:sub-label="opt.subLabel"
								:value="opt.value"
								:selected="isSelected(opt.value)"
							>
								<div class="flex flex-col min-w-0">
									<span class="truncate">{{ opt.label }}</span>
									<span
										v-if="opt.subLabel"
										class="truncate text-xs text-muted-foreground"
									>
										{{ opt.subLabel }}
									</span>
								</div>
							</slot>
							<UiIcon
								v-if="isSelected(opt.value)"
								name="check"
								size="14px"
								class="shrink-0 text-primary"
							/>
						</UiComboboxItem>

						<UiComboboxItem
							v-if="canCreateCustom"
							:value="trimmedQuery"
							class="text-primary"
							@select.prevent="handleSelectCustom"
						>
							<span class="flex items-center gap-1.5 truncate">
								<UiIcon name="plus" size="14px" class="shrink-0" />
								<span class="truncate">Use "{{ trimmedQuery }}"</span>
							</span>
						</UiComboboxItem>
					</UiComboboxGroup>

					<UiComboboxEmpty v-else>
						<slot name="empty">{{ props.emptyText }}</slot>
					</UiComboboxEmpty>
				</template>
			</UiComboboxList>
		</UiCombobox>

		<p v-if="props.helper" :class="cn('text-xs text-muted-foreground', props.helperClass)">
			{{ props.helper }}
		</p>

		<AppInputError v-model="showMessages" :messages="errorMessage" @close="handleAlertClose" />
	</div>
</template>

<style scoped>
.autocomplete-tooltip-scroll {
	--scrollbar-thumb: hsl(var(--primary-foreground) / 0.8);
	--scrollbar-track: transparent;
	scrollbar-width: thin;
	scrollbar-color: var(--scrollbar-thumb) var(--scrollbar-track);
}

@supports not (scrollbar-color: auto) {
	.autocomplete-tooltip-scroll {
		scrollbar-width: auto;
	}

	.autocomplete-tooltip-scroll::-webkit-scrollbar {
		width: 6px;
		height: 6px;
	}

	.autocomplete-tooltip-scroll::-webkit-scrollbar-thumb {
		background: var(--scrollbar-thumb);
		border-radius: 9999px;
	}

	.autocomplete-tooltip-scroll::-webkit-scrollbar-track {
		background: var(--scrollbar-track);
	}
}
</style>

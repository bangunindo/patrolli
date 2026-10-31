<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import AppInputSelect from '@/components/app/input/AppInputSelect.vue'
import AppInputText from '@/components/app/input/AppInputText.vue'
import AppInputError from '@/components/app/input/AppInputError.vue'
import type { FieldDescriptor, UiHint } from '@@/shared/types/dynamic-form.type'

const props = withDefaults(
	defineProps<{
		modelValue?: Record<string, any> | null
		descriptor: FieldDescriptor
		error?: string
		disabled?: boolean
		issues?: Record<string, string>
		hints?: Record<string, UiHint>
	}>(),
	{
		modelValue: () => ({}),
		error: '',
		disabled: false,
		issues: () => ({}),
		hints: () => ({})
	}
)

const emits = defineEmits<{
	'update:modelValue': [value: Record<string, any>]
}>()

const generateId = () => Math.random().toString(36).substring(2, 9)

const valueDescriptor = computed<FieldDescriptor | undefined>(() => {
	return props.descriptor.children?.[0]
})

const innerValueDescriptor = computed<FieldDescriptor | undefined>(() => {
	return valueDescriptor.value?.children?.[0]
})

// Category/table key options (e.g.: RECORD, SCOPE, RESOURCE)
const keyOptions = computed(() => {
	const h = props.descriptor.hints
	if (h?.keyOptions && Array.isArray(h.keyOptions) && h.keyOptions.length > 0) {
		return h.keyOptions
	}
	if (
		props.descriptor.options &&
		Array.isArray(props.descriptor.options) &&
		props.descriptor.options.length > 0
	) {
		return props.descriptor.options
	}
	return undefined
})

const hasKeyOptions = computed(() => !!keyOptions.value && keyOptions.value.length > 0)

// Determine category display mode if keyOptions are provided
const isArrayStringCategory = computed(() => {
	return hasKeyOptions.value && valueDescriptor.value?.widget === 'array-string'
})

const isRenameMappingCategory = computed(() => {
	return (
		hasKeyOptions.value &&
		valueDescriptor.value?.widget === 'record' &&
		(!innerValueDescriptor.value || innerValueDescriptor.value.widget === 'text')
	)
})

const isAddAttributeCategory = computed(() => {
	return (
		hasKeyOptions.value &&
		valueDescriptor.value?.widget === 'record' &&
		innerValueDescriptor.value?.widget === 'discriminated-union'
	)
})

const isCategorizedSection = computed(() => {
	return (
		isArrayStringCategory.value || isRenameMappingCategory.value || isAddAttributeCategory.value
	)
})

// ==========================================
// 1. STATE FOR CATEGORIZED MODE (SECTIONS)
// ==========================================

const isSyncing = ref(false)

// Array String: Record<Category, { id: string, text: string }[]>
const arrayItems = ref<Record<string, { id: string; text: string }[]>>({})

// Rename Map: Record<Category, { id: string, oldKey: string, newValue: string }[]>
const renameItems = ref<Record<string, { id: string; oldKey: string; newValue: string }[]>>({})

// Add Attribute: Record<Category, { id: string, keyName: string, type: string, value: any }[]>
const addAttrItems = ref<
	Record<string, { id: string; keyName: string; type: string; value: any }[]>
>({})

// Data type options for TransformValue
const unionOptions = computed(() => {
	const unionDesc = innerValueDescriptor.value
	if (unionDesc?.widget === 'discriminated-union' && unionDesc.options) {
		return unionDesc.options.map((opt) => ({
			label: opt.label.toUpperCase(),
			value: opt.value
		}))
	}
	return [
		{ label: 'STR', value: 'str' },
		{ label: 'INT', value: 'int' },
		{ label: 'DOUBLE', value: 'double' },
		{ label: 'BOOL', value: 'bool' }
	]
})

// Initialize categorized state from modelValue
const initCategorizedState = () => {
	if (!keyOptions.value) return
	const model = props.modelValue || {}

	if (isArrayStringCategory.value) {
		const next: Record<string, { id: string; text: string }[]> = {}
		for (const opt of keyOptions.value) {
			const existing = model[opt.value] || []
			next[opt.value] = Array.isArray(existing)
				? existing.map((val: string) => ({ id: generateId(), text: String(val) }))
				: []
		}
		arrayItems.value = next
	} else if (isRenameMappingCategory.value) {
		const next: Record<string, { id: string; oldKey: string; newValue: string }[]> = {}
		for (const opt of keyOptions.value) {
			const existing = model[opt.value] || {}
			next[opt.value] =
				typeof existing === 'object' && existing !== null && !Array.isArray(existing)
					? Object.entries(existing).map(([oldKey, newValue]) => ({
							id: generateId(),
							oldKey,
							newValue: String(newValue ?? '')
						}))
					: []
		}
		renameItems.value = next
	} else if (isAddAttributeCategory.value) {
		const next: Record<string, { id: string; keyName: string; type: string; value: any }[]> = {}
		for (const opt of keyOptions.value) {
			const existing = model[opt.value] || {}
			next[opt.value] =
				typeof existing === 'object' && existing !== null && !Array.isArray(existing)
					? Object.entries(existing).map(([keyName, transformObj]: [string, any]) => ({
							id: generateId(),
							keyName,
							type: transformObj?.type || 'str',
							value: transformObj?.value !== undefined ? transformObj.value : ''
						}))
					: []
		}
		addAttrItems.value = next
	}
}

const emitCategorizedUpdate = () => {
	isSyncing.value = true
	if (isArrayStringCategory.value) {
		const result: Record<string, string[]> = {}
		for (const opt of keyOptions.value || []) {
			const items = arrayItems.value[opt.value] || []
			const filtered = items.map((i) => i.text.trim()).filter((t) => t.length > 0)
			if (filtered.length > 0) {
				result[opt.value] = filtered
			}
		}
		emits('update:modelValue', result)
	} else if (isRenameMappingCategory.value) {
		const result: Record<string, Record<string, string>> = {}
		for (const opt of keyOptions.value || []) {
			const items = renameItems.value[opt.value] || []
			const mapObj: Record<string, string> = {}
			for (const item of items) {
				const oldKey = item.oldKey.trim()
				const newValue = item.newValue.trim()
				if (oldKey && newValue) {
					mapObj[oldKey] = newValue
				}
			}
			if (Object.keys(mapObj).length > 0) {
				result[opt.value] = mapObj
			}
		}
		emits('update:modelValue', result)
	} else if (isAddAttributeCategory.value) {
		const result: Record<string, Record<string, any>> = {}
		for (const opt of keyOptions.value || []) {
			const items = addAttrItems.value[opt.value] || []
			const innerMap: Record<string, any> = {}
			for (const item of items) {
				const trimmedKey = item.keyName.trim()
				if (!trimmedKey) continue

				let finalVal = item.value
				if (item.type === 'int') {
					const num = Number(finalVal)
					finalVal = isNaN(num) ? 0 : Math.trunc(num)
				} else if (item.type === 'double') {
					const num = Number(finalVal)
					finalVal = isNaN(num) ? 0.0 : num
				} else if (item.type === 'bool') {
					finalVal = Boolean(finalVal)
				} else {
					finalVal = String(finalVal ?? '')
				}

				innerMap[trimmedKey] = {
					type: item.type,
					value: finalVal
				}
			}
			if (Object.keys(innerMap).length > 0) {
				result[opt.value] = innerMap
			}
		}
		emits('update:modelValue', result)
	}
	nextTick(() => {
		isSyncing.value = false
	})
}

// Category button handlers
const addArrayItem = (category: string) => {
	if (props.disabled) return
	if (!arrayItems.value[category]) arrayItems.value[category] = []
	arrayItems.value[category].push({ id: generateId(), text: '' })
	emitCategorizedUpdate()
}

const removeArrayItem = (category: string, idx: number) => {
	if (props.disabled) return
	arrayItems.value[category]?.splice(idx, 1)
	emitCategorizedUpdate()
}

const addRenameItem = (category: string) => {
	if (props.disabled) return
	if (!renameItems.value[category]) renameItems.value[category] = []
	renameItems.value[category].push({ id: generateId(), oldKey: '', newValue: '' })
	emitCategorizedUpdate()
}

const removeRenameItem = (category: string, idx: number) => {
	if (props.disabled) return
	renameItems.value[category]?.splice(idx, 1)
	emitCategorizedUpdate()
}

const addAddAttrItem = (category: string) => {
	if (props.disabled) return
	if (!addAttrItems.value[category]) addAttrItems.value[category] = []
	addAttrItems.value[category].push({
		id: generateId(),
		keyName: '',
		type: 'str',
		value: ''
	})
	emitCategorizedUpdate()
}

const removeAddAttrItem = (category: string, idx: number) => {
	if (props.disabled) return
	addAttrItems.value[category]?.splice(idx, 1)
	emitCategorizedUpdate()
}

const handleTransformTypeChange = (item: any) => {
	if (item.type === 'bool') item.value = false
	else if (item.type === 'str') item.value = ''
	else item.value = 0
	emitCategorizedUpdate()
}

const onTypeChange = (item: any, newType: any) => {
	item.type = newType
	handleTransformTypeChange(item)
}

// ==========================================
// 2. STATE FOR FREE-FORM ENTRY (GENERIC)
// ==========================================

interface GenericEntry {
	id: string
	key: string
	value: any
}

const genericEntries = ref<GenericEntry[]>([])

const initGenericState = () => {
	const map = props.modelValue || {}
	if (typeof map !== 'object' || Array.isArray(map)) {
		genericEntries.value = []
		return
	}
	genericEntries.value = Object.entries(map).map(([k, v]) => ({
		id: generateId(),
		key: k,
		value: v !== undefined ? structuredClone(v) : ''
	}))
}

const emitGenericUpdate = () => {
	isSyncing.value = true
	const result: Record<string, any> = {}
	for (const e of genericEntries.value) {
		const k = e.key.trim()
		if (k) result[k] = e.value
	}
	emits('update:modelValue', result)
	nextTick(() => {
		isSyncing.value = false
	})
}

const addGenericEntry = () => {
	if (props.disabled) return
	genericEntries.value.push({
		id: generateId(),
		key: '',
		value: ''
	})
	emitGenericUpdate()
}

const removeGenericEntry = (idx: number) => {
	if (props.disabled) return
	genericEntries.value.splice(idx, 1)
	emitGenericUpdate()
}

// ==========================================
// 3. ROOT WATCHER SYNCHRONIZATION
// ==========================================

const initAll = () => {
	if (isCategorizedSection.value) {
		initCategorizedState()
	} else {
		initGenericState()
	}
}

initAll()

watch(
	() => props.modelValue,
	() => {
		if (isSyncing.value) return
		initAll()
	},
	{ deep: true }
)
</script>

<template>
	<div
		class="flex flex-col gap-2 p-1"
		:data-invalid="!!props.error"
		:class="{ 'has-error': !!props.error }"
	>
		<!-- ======================================================== -->
		<!-- CASE 1: CATEGORIZED SECTIONS (RECORD, SCOPE, RESOURCE)   -->
		<!-- ======================================================== -->
		<template v-if="isCategorizedSection">
			<div
				v-for="opt in keyOptions"
				:key="opt.value"
				class="mb-6 p-4 border border-gray-200 rounded-lg bg-white"
			>
				<div class="text-sm font-bold text-gray-700 uppercase mb-3">
					{{ opt.label }}
					{{
						isArrayStringCategory
							? 'Configuration'
							: isRenameMappingCategory
								? 'Rename Mapping'
								: 'Configuration'
					}}
				</div>

				<!-- A. Drop Attributes (Array of Strings) -->
				<template v-if="isArrayStringCategory">
					<div
						v-for="(item, idx) in arrayItems[opt.value] || []"
						:key="item.id"
						class="flex gap-2 items-center mt-2"
					>
						<AppInputText
							v-model="item.text"
							:placeholder="`Input value ${opt.label.toLowerCase()}`"
							size="sm"
							class="flex-1"
							:disabled="props.disabled"
							@update:model-value="emitCategorizedUpdate"
						/>
						<UiButton
							type="button"
							variant="ghost"
							size="icon"
							class="h-8 w-8 text-destructive hover:bg-destructive/10 shrink-0"
							:disabled="props.disabled"
							@click="removeArrayItem(opt.value, idx)"
						>
							<UiIcon name="minus" size="16px" />
						</UiButton>
					</div>

					<div class="mt-3">
						<UiButton
							type="button"
							variant="ghost"
							size="sm"
							class="gap-1 text-primary hover:text-primary"
							:disabled="props.disabled"
							@click="addArrayItem(opt.value)"
						>
							<UiIcon name="plus" size="14px" />
							<span>Add {{ opt.label }} Key</span>
						</UiButton>
					</div>
				</template>

				<!-- B. Rename Attributes (Pair Old Key -> New Value) -->
				<template v-else-if="isRenameMappingCategory">
					<div
						v-for="(item, idx) in renameItems[opt.value] || []"
						:key="item.id"
						class="flex gap-2 items-center mt-2"
					>
						<AppInputText
							v-model="item.oldKey"
							:placeholder="`Old Value (${opt.label.toLowerCase()})`"
							size="sm"
							class="flex-1"
							:disabled="props.disabled"
							@update:model-value="emitCategorizedUpdate"
						/>
						<span class="text-gray-400 text-xs">-&gt;</span>
						<AppInputText
							v-model="item.newValue"
							:placeholder="`New Value (${opt.label.toLowerCase()})`"
							size="sm"
							class="flex-1"
							:disabled="props.disabled"
							@update:model-value="emitCategorizedUpdate"
						/>
						<UiButton
							type="button"
							variant="ghost"
							size="icon"
							class="h-8 w-8 text-destructive hover:bg-destructive/10 shrink-0"
							:disabled="props.disabled"
							@click="removeRenameItem(opt.value, idx)"
						>
							<UiIcon name="minus" size="16px" />
						</UiButton>
					</div>

					<div class="mt-3">
						<UiButton
							type="button"
							variant="ghost"
							size="sm"
							class="gap-1 text-primary hover:text-primary"
							:disabled="props.disabled"
							@click="addRenameItem(opt.value)"
						>
							<UiIcon name="plus" size="14px" />
							<span>Add {{ opt.label }} Map</span>
						</UiButton>
					</div>
				</template>

				<!-- C. Add Attributes (Key Name + Value Type + Value) -->
				<template v-else-if="isAddAttributeCategory">
					<div
						v-for="(item, idx) in addAttrItems[opt.value] || []"
						:key="item.id"
						class="flex flex-col gap-2 p-3 bg-muted/40 rounded-md border border-border mb-3"
					>
						<AppInputText
							v-model="item.keyName"
							label="Attribute Key Name (Inner Key)"
							placeholder="Enter attribute key name"
							required
							:disabled="props.disabled"
							@update:model-value="emitCategorizedUpdate"
						/>

						<AppInputSelect
							:model-value="item.type"
							label="Config Type"
							label-class="font-semibold"
							:options="unionOptions"
							label-key="label"
							value-key="value"
							required
							:disabled="props.disabled"
							@update:model-value="onTypeChange(item, $event)"
						/>

						<div v-if="item.type && item.type !== 'bool'">
							<AppInputText
								v-model="item.value"
								:type="item.type === 'str' ? 'text' : 'number'"
								:step="item.type === 'double' ? 'any' : undefined"
								label="Value"
								:placeholder="`Input ${item.type} value`"
								required
								:disabled="props.disabled"
								@update:model-value="emitCategorizedUpdate"
							/>
						</div>
						<div v-else-if="item.type === 'bool'" class="flex flex-col gap-1">
							<label class="font-semibold block text-sm">
								Value <span class="text-destructive">*</span>
							</label>
							<div class="flex items-center gap-2">
								<UiSwitch
									v-model:checked="item.value"
									:disabled="props.disabled"
									@update:checked="emitCategorizedUpdate"
								/>
							</div>
						</div>

						<div class="flex justify-end mt-2">
							<UiButton
								type="button"
								variant="ghost"
								size="sm"
								class="text-destructive hover:bg-destructive/10 gap-1 text-xs"
								:disabled="props.disabled"
								@click="removeAddAttrItem(opt.value, idx)"
							>
								<UiIcon name="minus" size="14px" />
								<span>Remove Configuration</span>
							</UiButton>
						</div>
					</div>

					<div class="mt-3">
						<UiButton
							type="button"
							variant="ghost"
							size="sm"
							class="gap-1 text-primary hover:text-primary"
							:disabled="props.disabled"
							@click="addAddAttrItem(opt.value)"
						>
							<UiIcon name="plus" size="14px" />
							<span>Add {{ opt.label }} Key</span>
						</UiButton>
					</div>
				</template>
			</div>
		</template>

		<!-- ======================================================== -->
		<!-- CASE 2: GENERIC FREE-FORM RECORD (Arbitrary Key-Value)    -->
		<!-- ======================================================== -->
		<template v-else>
			<div class="flex justify-between items-center mb-1">
				<label class="font-semibold block text-sm">
					{{ props.descriptor.label }}
					<span v-if="props.descriptor.required" class="text-destructive">*</span>
				</label>
			</div>

			<div
				v-for="(entry, idx) in genericEntries"
				:key="entry.id"
				class="flex gap-2 items-center mb-2"
			>
				<UiInput
					v-model="entry.key"
					placeholder="Key"
					class="h-8 text-xs flex-1"
					:disabled="props.disabled"
					@input="emitGenericUpdate"
				/>
				<UiInput
					v-model="entry.value"
					placeholder="Value"
					class="h-8 text-xs flex-1"
					:disabled="props.disabled"
					@input="emitGenericUpdate"
				/>
				<UiButton
					type="button"
					variant="outline"
					size="icon"
					class="h-8 w-8 text-destructive border-destructive/30 hover:bg-destructive/10 shrink-0"
					:disabled="props.disabled"
					@click="removeGenericEntry(idx)"
				>
					<UiIcon name="minus" size="14px" />
				</UiButton>
			</div>

			<div>
				<button
					v-if="!props.disabled"
					type="button"
					class="text-xs font-semibold text-primary hover:underline flex items-center gap-1 cursor-pointer"
					@click="addGenericEntry"
				>
					<UiIcon name="plus" size="14px" />
					<span>Add {{ props.descriptor.label }} Entry</span>
				</button>
			</div>
		</template>

		<p v-if="props.descriptor.description" class="text-xs text-gray-500">
			{{ props.descriptor.description }}
		</p>
		<AppInputError :messages="props.error" />
	</div>
</template>

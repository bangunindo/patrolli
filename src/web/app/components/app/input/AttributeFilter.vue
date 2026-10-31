<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed, watch } from 'vue'
import AppInputSelect from '@/components/app/input/AppInputSelect.vue'
import AppInputText from '@/components/app/input/AppInputText.vue'
import AppInputError from '@/components/app/input/AppInputError.vue'
import FieldGroup from '@/components/app/input/FieldGroup.vue'
import { AttributeTable, KeyValueOp } from '@@/shared/generated/patrolliAPI.schemas.type'
import type { FieldDescriptor, UiHint } from '@@/shared/types/dynamic-form.type'

defineOptions({
	name: 'AppInputAttributeFilter'
})

const props = withDefaults(
	defineProps<{
		modelValue?: any
		descriptor?: FieldDescriptor
		error?: string
		disabled?: boolean
		issues?: Record<string, string>
		hints?: Record<string, UiHint>
		root?: boolean
	}>(),
	{
		modelValue: undefined,
		error: '',
		disabled: false,
		issues: () => ({}),
		hints: () => ({}),
		root: true
	}
)

const emits = defineEmits<{
	'update:modelValue': [value: any]
}>()

const tables = Object.keys(AttributeTable.Enum).map((val) => ({
	label: val.toUpperCase(),
	value: val
}))

const operators = Object.keys(KeyValueOp.Enum).map((val) => ({
	label: val,
	value: val
}))

const conditionOptions = [
	{ label: 'MATCH (Single Condition)', value: 'match' },
	{ label: 'AND (All conditions must match)', value: 'and' },
	{ label: 'OR (Any condition matches)', value: 'or' }
]

const valueTypes = [
	{ label: 'STRING', value: 'string' },
	{ label: 'INTEGER', value: 'integer' },
	{ label: 'FLOAT', value: 'float' }
]

const createDefaultMatch = () => ({
	key: '',
	key_is_regex: false,
	op: '=',
	table: 'record',
	value: { string: '' }
})

// Normalize internal representation to ensure it is always valid
const innerConfig = computed<any>({
	get() {
		if (!props.modelValue || typeof props.modelValue !== 'object') {
			return { match: createDefaultMatch() }
		}
		if ('and' in props.modelValue || 'or' in props.modelValue || 'match' in props.modelValue) {
			return props.modelValue
		}
		return { match: createDefaultMatch() }
	},
	set(val) {
		emits('update:modelValue', val)
	}
})

// Watcher to initialize defaults if prop is empty
watch(
	() => props.modelValue,
	(val) => {
		if (
			!val ||
			typeof val !== 'object' ||
			(!('and' in val) && !('or' in val) && !('match' in val))
		) {
			emits('update:modelValue', { match: createDefaultMatch() })
		}
	},
	{ immediate: true, deep: true }
)

// Deep watcher to ensure any sub-property change triggers an emit
watch(
	() => innerConfig.value,
	(val) => {
		emits('update:modelValue', val)
	},
	{ deep: true }
)

const formConfigType = computed({
	get() {
		if (!innerConfig.value) return 'match'
		if ('and' in innerConfig.value) return 'and'
		if ('or' in innerConfig.value) return 'or'
		return 'match'
	},
	set(val) {
		if (val === 'match') {
			innerConfig.value = { match: createDefaultMatch() }
		} else if (val === 'and') {
			innerConfig.value = {
				and: [createDefaultMatch()]
			}
		} else if (val === 'or') {
			innerConfig.value = {
				or: [createDefaultMatch()]
			}
		}
	}
})

// Value type selection & value casting
const selectedValueType = computed({
	get() {
		if (!innerConfig.value?.match?.value) return 'string'
		const keys = Object.keys(innerConfig.value.match.value)
		return keys[0] || 'string'
	},
	set(newType) {
		if (!innerConfig.value?.match) return
		const currentRaw = matchValueInput.value
		const updated = {
			...innerConfig.value,
			match: {
				...innerConfig.value.match
			}
		}
		if (newType === 'integer') {
			const parsed = parseInt(String(currentRaw))
			updated.match.value = { integer: isNaN(parsed) ? 0 : parsed }
		} else if (newType === 'float') {
			const parsed = parseFloat(String(currentRaw))
			updated.match.value = { float: isNaN(parsed) ? 0 : parsed }
		} else {
			updated.match.value = { string: String(currentRaw ?? '') }
		}
		innerConfig.value = updated
	}
})

const matchValueInput = computed({
	get() {
		if (!innerConfig.value?.match?.value) return ''
		const type = selectedValueType.value
		const val = innerConfig.value.match.value[type]
		return val !== undefined && val !== null ? val : ''
	},
	set(newVal) {
		if (!innerConfig.value?.match) return
		const type = selectedValueType.value
		const updated = {
			...innerConfig.value,
			match: {
				...innerConfig.value.match,
				value: {} as Record<string, any>
			}
		}

		if (type === 'integer') {
			if (newVal === '' || newVal === '-') {
				updated.match.value = { [type]: newVal }
			} else {
				const num = Number(newVal)
				updated.match.value = { [type]: isNaN(num) ? newVal : Math.trunc(num) }
			}
		} else if (type === 'float') {
			if (
				newVal === '' ||
				(typeof newVal === 'string' && newVal.endsWith('.')) ||
				newVal === '-'
			) {
				updated.match.value = { [type]: newVal }
			} else {
				const num = Number(newVal)
				updated.match.value = { [type]: isNaN(num) ? newVal : num }
			}
		} else {
			updated.match.value = { [type]: newVal }
		}
		innerConfig.value = updated
	}
})

const addCondition = () => {
	const type = formConfigType.value
	if (type === 'and' || type === 'or') {
		const updated = { ...innerConfig.value }
		if (!Array.isArray(updated[type])) {
			updated[type] = []
		}
		updated[type] = [...updated[type], { match: createDefaultMatch() }]
		innerConfig.value = updated
	}
}

const removeCondition = (index: number) => {
	const type = formConfigType.value
	if ((type === 'and' || type === 'or') && Array.isArray(innerConfig.value?.[type])) {
		const updated = { ...innerConfig.value }
		const list = [...updated[type]]
		list.splice(index, 1)
		updated[type] = list
		innerConfig.value = updated
	}
}

const updateChildCondition = (index: number, val: any) => {
	const type = formConfigType.value
	if ((type === 'and' || type === 'or') && Array.isArray(innerConfig.value?.[type])) {
		const updated = { ...innerConfig.value }
		const list = [...updated[type]]
		list[index] = val
		updated[type] = list
		innerConfig.value = updated
	}
}
</script>

<template>
	<div
		:class="{
			'pt-1 rounded mb-3': props.root,
			'p-0': !props.root,
			'has-error': !!props.error
		}"
		:data-invalid="!!props.error"
		v-if="innerConfig"
	>
		<!-- Error banner if root and error exists -->
		<AppInputError v-if="props.root" :messages="props.error" />

		<AppInputSelect
			v-model="formConfigType"
			variant="outline"
			placeholder="Select condition"
			label="Condition"
			label-class="font-semibold"
			:options="conditionOptions"
			:disabled="props.disabled"
		/>

		<div class="pt-3">
			<!-- AND / OR Branch -->
			<template v-if="formConfigType === 'and' || formConfigType === 'or'">
				<div class="flex flex-col gap-3">
					<div class="flex justify-between items-center">
						<span class="font-semibold text-sm uppercase tracking-wider text-gray-500">
							{{ formConfigType }} Group
						</span>
						<UiButton
							type="button"
							size="sm"
							variant="ghost"
							class="gap-1 text-primary hover:text-primary"
							:disabled="props.disabled"
							@click="addCondition"
						>
							<UiIcon name="plus" size="14px" />
							<span>Add Condition</span>
						</UiButton>
					</div>

					<FieldGroup
						:label="`${formConfigType.toUpperCase()} Conditions`"
						v-if="innerConfig[formConfigType]?.length > 0"
					>
						<div class="flex flex-col gap-4 mt-2">
							<div
								v-for="(child, index) in innerConfig[formConfigType]"
								:key="index"
								class="p-3 border border-border rounded-lg bg-muted/30"
							>
								<AppInputAttributeFilter
									:model-value="child"
									:root="false"
									:disabled="props.disabled"
									:issues="props.issues"
									@update:model-value="
										updateChildCondition(Number(index), $event)
									"
								/>

								<div class="flex justify-end mt-2">
									<UiButton
										type="button"
										size="sm"
										variant="ghost"
										class="gap-1 text-destructive hover:bg-destructive/10 text-xs"
										:disabled="props.disabled"
										@click="removeCondition(Number(index))"
									>
										<UiIcon name="minus" size="14px" />
										<span>Remove Condition</span>
									</UiButton>
								</div>
							</div>
						</div>
					</FieldGroup>
				</div>
			</template>

			<!-- MATCH Branch -->
			<template v-if="formConfigType === 'match' && innerConfig.match">
				<FieldGroup :label="props.root ? 'Match Condition' : undefined">
					<div class="flex flex-col gap-2 mt-3">
						<AppInputText
							v-model="innerConfig.match.key"
							placeholder="e.g. http.status_code, service.name"
							label="Key"
							label-class="font-semibold"
							required
							:disabled="props.disabled"
						/>
					</div>

					<div class="flex flex-col gap-2 mt-3 mb-2">
						<label class="font-semibold block text-sm">
							Key Is Regex <span class="text-destructive">*</span>
						</label>
						<div class="flex items-center gap-2">
							<UiSwitch
								v-model:checked="innerConfig.match.key_is_regex"
								:disabled="props.disabled"
							/>
							<span class="text-sm font-normal text-muted-foreground">
								{{ innerConfig.match.key_is_regex ? 'Regex' : 'Non Regex' }}
							</span>
						</div>
					</div>

					<div class="flex flex-col gap-2 mt-3">
						<AppInputSelect
							v-model="innerConfig.match.op"
							variant="outline"
							placeholder="Select operator"
							label="Operator"
							label-class="font-semibold"
							:options="operators"
							required
							:disabled="props.disabled"
						/>
					</div>

					<div class="flex flex-col gap-2 mt-3">
						<AppInputSelect
							v-model="innerConfig.match.table"
							variant="outline"
							placeholder="Select table"
							label="Table"
							label-class="font-semibold"
							:options="tables"
							required
							:disabled="props.disabled"
						/>
					</div>

					<div class="flex flex-col gap-2 mt-3">
						<AppInputSelect
							v-model="selectedValueType"
							variant="outline"
							placeholder="Select value type"
							label="Value Type"
							label-class="font-semibold"
							:options="valueTypes"
							required
							:disabled="props.disabled"
						/>
					</div>

					<div class="flex flex-col gap-2 mt-3">
						<AppInputText
							:type="
								selectedValueType === 'integer' || selectedValueType === 'float'
									? 'number'
									: 'text'
							"
							v-model="matchValueInput"
							placeholder="Value"
							label="Value"
							label-class="font-semibold"
							required
							:disabled="props.disabled"
						/>
					</div>
				</FieldGroup>
			</template>
		</div>
		<AppInputError v-if="props.root && props.error" :messages="props.error" />
	</div>
</template>

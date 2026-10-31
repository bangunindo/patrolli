<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed, inject, nextTick, ref, watch, type ComputedRef } from 'vue'
import { rendererProps, useJsonFormsControl } from '@jsonforms/vue'
import type { ControlElement } from '@jsonforms/core'
import AppInputSelect from '@/components/app/input/AppInputSelect.vue'
import AppInputError from '@/components/app/input/AppInputError.vue'
import { resolveHintForPath } from '@/app/services/forms/hints-to-uischema'
import type { UiHint } from '@@/shared/types/dynamic-form.type'
import { formatFieldLabel } from '@/app/services/forms/path-helper'

const props = defineProps({
	...rendererProps<ControlElement>()
})

const { control, handleChange } = useJsonFormsControl(props)
const formHintsRef = inject<ComputedRef<Record<string, UiHint>> | undefined>('formHints', undefined)
const formHints = computed(() => formHintsRef?.value || {})

const leafName = computed(() => {
	const p = control.value.path || ''
	const parts = p.split('.')
	return parts[parts.length - 1] || p
})

const selfHint = computed(() => {
	return resolveHintForPath(control.value.path, formHints.value, (control.value as any).rootData)
})

const fieldLabel = computed(() => {
	if (selfHint.value?.label) return selfHint.value.label
	if (control.value.label) return control.value.label
	return leafName.value ? formatFieldLabel(leafName.value) : 'Attributes'
})

const generateId = () => Math.random().toString(36).substring(2, 9)

export type LogAttrType = 'String' | 'Int64' | 'Double' | 'Bool' | 'Null'

export interface LogAttrItem {
	id: string
	key: string
	type: LogAttrType
	value: any
}

const typeOptions: { label: string; value: LogAttrType }[] = [
	{ label: 'String', value: 'String' },
	{ label: 'Int64', value: 'Int64' },
	{ label: 'Double', value: 'Double' },
	{ label: 'Bool', value: 'Bool' },
	{ label: 'Null', value: 'Null' }
]

const parseTupleArray = (data: any): LogAttrItem[] => {
	if (!data) return []

	if (Array.isArray(data)) {
		return data.map((item) => {
			if (!Array.isArray(item) || item.length < 2) {
				return {
					id: generateId(),
					key: '',
					type: 'String',
					value: ''
				}
			}

			const keyObj = item[0]
			const key =
				keyObj?.String ?? keyObj?.SmolStr ?? (typeof keyObj === 'string' ? keyObj : '')

			const valObj = item[1]
			let type: LogAttrType = 'String'
			let value: any = ''

			if (valObj === 'Null' || valObj === null) {
				type = 'Null'
				value = null
			} else if (valObj?.Bool !== undefined) {
				type = 'Bool'
				value = Boolean(valObj.Bool)
			} else if (valObj?.Int64 !== undefined) {
				type = 'Int64'
				value = Number(valObj.Int64)
			} else if (valObj?.Double !== undefined) {
				type = 'Double'
				value = Number(valObj.Double)
			} else if (valObj?.String !== undefined) {
				type = 'String'
				const inner = valObj.String
				value =
					typeof inner === 'object'
						? (inner?.String ?? inner?.SmolStr ?? '')
						: String(inner)
			} else if (typeof valObj === 'boolean') {
				type = 'Bool'
				value = valObj
			} else if (typeof valObj === 'number') {
				type = Number.isInteger(valObj) ? 'Int64' : 'Double'
				value = valObj
			} else if (typeof valObj === 'string') {
				type = 'String'
				value = valObj
			} else {
				type = 'String'
				value = String(valObj ?? '')
			}

			return {
				id: generateId(),
				key: String(key ?? ''),
				type,
				value
			}
		})
	}

	if (typeof data === 'object') {
		return Object.entries(data).map(([k, v]) => ({
			id: generateId(),
			key: k,
			type: 'String',
			value: String(v ?? '')
		}))
	}

	return []
}

const serializeTupleArray = (items: LogAttrItem[]): any[] => {
	return items.map((item) => {
		const keyObj = { String: item.key }
		let valObj: any = 'Null'

		if (item.type === 'String') {
			valObj = { String: { String: String(item.value ?? '') } }
		} else if (item.type === 'Int64') {
			const num = Number(item.value)
			valObj = { Int64: isNaN(num) ? 0 : Math.trunc(num) }
		} else if (item.type === 'Double') {
			const num = Number(item.value)
			valObj = { Double: isNaN(num) ? 0.0 : num }
		} else if (item.type === 'Bool') {
			valObj = { Bool: Boolean(item.value) }
		} else if (item.type === 'Null') {
			valObj = 'Null'
		}

		return [keyObj, valObj]
	})
}

const rows = ref<LogAttrItem[]>(parseTupleArray(control.value.data))
const isSyncing = ref(false)

const emitUpdate = () => {
	isSyncing.value = true
	const serialized = serializeTupleArray(rows.value)
	handleChange(control.value.path, serialized)
	nextTick(() => {
		isSyncing.value = false
	})
}

const onTypeChange = (row: LogAttrItem, newType: any) => {
	row.type = newType
	if (newType === 'Bool') {
		row.value = false
	} else if (newType === 'Int64' || newType === 'Double') {
		row.value = 0
	} else if (newType === 'Null') {
		row.value = null
	} else {
		row.value = ''
	}
	emitUpdate()
}

const addRow = () => {
	if (!control.value.enabled) return
	rows.value.push({
		id: generateId(),
		key: '',
		type: 'String',
		value: ''
	})
	emitUpdate()
}

const removeRow = (idx: number) => {
	if (!control.value.enabled) return
	rows.value.splice(idx, 1)
	emitUpdate()
}

const onFieldChange = () => {
	emitUpdate()
}

watch(
	() => control.value.data,
	(newVal) => {
		if (isSyncing.value) return
		rows.value = parseTupleArray(newVal)
	},
	{ deep: true }
)
</script>

<template>
	<div
		v-if="control.visible"
		:data-invalid="!!control.errors"
		:class="{ 'has-error': !!control.errors }"
		class="w-full flex flex-col gap-2"
	>
		<!-- Header / Label -->
		<div class="flex justify-between items-center mb-1">
			<label v-if="fieldLabel" class="font-semibold block text-sm">
				{{ fieldLabel }}
				<span v-if="control.required" class="text-destructive">*</span>
			</label>
			<span v-if="rows.length > 0" class="text-xs text-gray-500">
				{{ rows.length }} attribute{{ rows.length !== 1 ? 's' : '' }}
			</span>
		</div>

		<!-- Rows -->
		<div v-for="(row, idx) in rows" :key="row.id" class="flex gap-2 items-center">
			<!-- Attribute Key -->
			<div class="flex-1 min-w-0">
				<UiInput
					v-model="row.key"
					placeholder="Key"
					class="h-8 text-xs"
					:disabled="!control.enabled"
					@input="onFieldChange"
				/>
			</div>

			<!-- Attribute Type -->
			<div class="w-28 flex-shrink-0">
				<AppInputSelect
					:model-value="row.type"
					:options="typeOptions"
					label-key="label"
					value-key="value"
					size="sm"
					:disabled="!control.enabled"
					@update:model-value="onTypeChange(row, $event)"
				/>
			</div>

			<!-- Attribute Value -->
			<div class="flex-1 min-w-0">
				<UiInput
					v-if="row.type === 'String'"
					v-model="row.value"
					placeholder="Value"
					class="h-8 text-xs"
					:disabled="!control.enabled"
					@input="onFieldChange"
				/>
				<UiInput
					v-else-if="row.type === 'Int64'"
					v-model="row.value"
					type="number"
					step="1"
					placeholder="0"
					class="h-8 text-xs"
					:disabled="!control.enabled"
					@input="onFieldChange"
				/>
				<UiInput
					v-else-if="row.type === 'Double'"
					v-model="row.value"
					type="number"
					step="any"
					placeholder="0.0"
					class="h-8 text-xs"
					:disabled="!control.enabled"
					@input="onFieldChange"
				/>
				<div v-else-if="row.type === 'Bool'" class="flex items-center h-full px-2 gap-2">
					<UiSwitch
						:model-value="!!row.value"
						:disabled="!control.enabled"
						@update:model-value="
							(val) => {
								row.value = val
								onFieldChange()
							}
						"
					/>
					<span class="text-xs text-muted-foreground font-mono">
						{{ row.value ? 'true' : 'false' }}
					</span>
				</div>
				<div
					v-else-if="row.type === 'Null'"
					class="flex items-center h-full px-2 text-xs text-muted-foreground italic"
				>
					(null)
				</div>
			</div>

			<!-- Action: Remove -->
			<UiButton
				type="button"
				variant="outline"
				size="icon"
				class="h-8 w-8 text-destructive hover:text-destructive"
				:disabled="!control.enabled"
				@click="removeRow(idx)"
			>
				<UiIcon name="minus" class="w-3.5 h-3.5" />
			</UiButton>
		</div>

		<!-- Action: Add Attribute -->
		<div>
			<UiButton
				v-if="control.enabled"
				type="button"
				variant="ghost"
				size="sm"
				class="h-7 px-2 text-xs text-primary flex items-center gap-1.5"
				@click="addRow"
			>
				<UiIcon name="plus" class="w-3.5 h-3.5" />
				<span>Add Attribute</span>
			</UiButton>
		</div>

		<AppInputError :messages="control.errors" />
	</div>
</template>

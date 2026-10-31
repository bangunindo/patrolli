<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed, inject, watchEffect, type ComputedRef } from 'vue'
import { rendererProps, useJsonFormsControl, DispatchRenderer } from '@jsonforms/vue'
import type { ControlElement, JsonSchema, UISchemaElement } from '@jsonforms/core'
import AppInputSelect from '@/components/app/input/AppInputSelect.vue'
import AppInputError from '@/components/app/input/AppInputError.vue'
import UiCheckbox from '@/components/ui/checkbox/Checkbox.vue'
import { createControlElement, resolveHintForPath } from '@/app/services/forms/hints-to-uischema'
import type { UiHint } from '@@/shared/types/dynamic-form.type'
import { formatFieldLabel } from '@/app/services/forms/path-helper'
import CustomSlotRenderer from './CustomSlotRenderer.vue'

const props = defineProps({
	...rendererProps<ControlElement>()
})

const { control, handleChange } = useJsonFormsControl(props)
const slots = inject<Record<string, any>>('dynamicZodFormSlots', {})
const formHintsRef = inject<ComputedRef<Record<string, UiHint>> | undefined>('formHints', undefined)
const formHints = computed(() => formHintsRef?.value || {})

const leafName = computed(() => {
	const p = control.value.path || ''
	const parts = p.split('.')
	return parts[parts.length - 1] || p
})

const customSlot = computed(() => {
	const pathKey = control.value.path
	return slots[`field:${pathKey}`] || slots[`field:${leafName.value}`] || null
})

const onInput = (val: any) => {
	handleChange(control.value.path, val)
}

const slotProps = computed(() => ({
	value: control.value.data,
	modelValue: control.value.data,
	'model-value': control.value.data,
	error: control.value.errors,
	updateModelValue: onInput,
	'update-model-value': onInput,
	handleChange: onInput,
	'handle-change': onInput
}))

const selfHint = computed(() => {
	return resolveHintForPath(control.value.path, formHints.value, (control.value as any).rootData)
})

// Flatten branches recursively to handle nested anyOf/oneOf (e.g. nullable wrapper anyOf: [null, anyOf: [Basic, Bearer]])
const rawBranches = computed<JsonSchema[]>(() => {
	const s = control.value.schema as any
	const list = s.anyOf || s.oneOf || []
	const flat: JsonSchema[] = []

	const recurse = (arr: any[]) => {
		for (const item of arr) {
			if (item.type === 'null') {
				flat.push(item)
			} else if (item.anyOf && Array.isArray(item.anyOf)) {
				recurse(item.anyOf)
			} else if (item.oneOf && Array.isArray(item.oneOf)) {
				recurse(item.oneOf)
			} else {
				flat.push(item)
			}
		}
	}
	recurse(list)
	return flat
})

const discriminatorKey = computed(() => {
	const explicit =
		control.value.uischema?.options?.discriminatorField || selfHint.value?.discriminatorField
	if (explicit) return explicit

	// Auto-detect from branches
	const candidates = ['type', 'module', 'kind', 'method', 'strategy']
	for (const candidate of candidates) {
		const found = rawBranches.value.some((b: any) => b.properties && candidate in b.properties)
		if (found) return candidate
	}

	return 'type'
})

interface NormalizedBranch {
	key: string
	label: string
	schema: JsonSchema
	isExternalTagged: boolean
	isNull: boolean
	externalPropertyKey?: string
}

const normalizedBranches = computed<NormalizedBranch[]>(() => {
	const res: NormalizedBranch[] = []
	for (const b of rawBranches.value as any[]) {
		if (b.type === 'null') {
			res.push({
				key: 'none',
				label: 'None',
				schema: b,
				isExternalTagged: false,
				isNull: true
			})
			continue
		}

		// 1. Internal tagged: has discriminator property (e.g. type: 'snmp_v1' or 'http')
		const discProp = b.properties?.[discriminatorKey.value]
		if (discProp) {
			const val = discProp.enum?.[0] ?? discProp.const ?? b.title ?? 'unknown'
			const label = b.title || val.toUpperCase().replace('_', ' ')
			res.push({
				key: String(val),
				label,
				schema: b,
				isExternalTagged: false,
				isNull: false
			})
			continue
		}

		// 2. External tagged: single property wrapping variant (e.g. { properties: { Basic: ... } } or { properties: { Bearer: ... } })
		if (b.properties && typeof b.properties === 'object') {
			const propKeys = Object.keys(b.properties)
			if (propKeys.length === 1) {
				const tagKey = propKeys[0]
				res.push({
					key: tagKey,
					label: b.title || tagKey,
					schema: b,
					isExternalTagged: true,
					isNull: false,
					externalPropertyKey: tagKey
				})
				continue
			}
		}

		// Fallback
		const val = b.title || 'unknown'
		res.push({
			key: String(val),
			label: String(val),
			schema: b,
			isExternalTagged: false,
			isNull: false
		})
	}

	// Apply optionsOrder if specified in hints
	const order = control.value.uischema?.options?.optionsOrder || selfHint.value?.optionsOrder
	if (Array.isArray(order) && order.length > 0) {
		res.sort((a, b) => {
			const idxA = order.indexOf(a.key)
			const idxB = order.indexOf(b.key)
			const posA = idxA === -1 ? 999 : idxA
			const posB = idxB === -1 ? 999 : idxB
			return posA - posB
		})
	}

	return res
})

const branchOptions = computed(() => {
	return normalizedBranches.value.map((nb) => ({
		label: nb.label,
		value: nb.key
	}))
})

const currentBranchValue = computed(() => {
	const data = control.value.data
	if (data === null || data === undefined) {
		const hasNullBranch = normalizedBranches.value.some((b) => b.isNull)
		if (hasNullBranch) return 'none'
	}

	if (data && typeof data === 'object') {
		// Internal tagged
		if (discriminatorKey.value in data && data[discriminatorKey.value] !== undefined) {
			return String(data[discriminatorKey.value])
		}
		if (discriminatorKey.value === 'type' && 'module' in data && data.module !== undefined) {
			return String(data.module)
		}
		if (discriminatorKey.value === 'module' && 'type' in data && data.type !== undefined) {
			return String(data.type)
		}
		// External tagged
		for (const nb of normalizedBranches.value) {
			if (nb.isExternalTagged && nb.externalPropertyKey && nb.externalPropertyKey in data) {
				return nb.key
			}
		}
	}

	return (
		control.value.uischema?.options?.discriminatorDefault ||
		selfHint.value?.discriminatorDefault ||
		branchOptions.value[0]?.value ||
		''
	)
})

const activeBranchInfo = computed<NormalizedBranch | null>(() => {
	const val = currentBranchValue.value
	return (
		normalizedBranches.value.find((b) => b.key === val) || normalizedBranches.value[0] || null
	)
})

const activeBranch = computed<JsonSchema | null>(() => {
	return activeBranchInfo.value?.schema || null
})

const activeBranchUiSchema = computed<UISchemaElement | null>(() => {
	const info = isOptionalObjectUnion.value ? singleObjectBranch.value : activeBranchInfo.value
	if (!info || info.isNull || !info.schema.properties) return null

	const subElements: ControlElement[] = []
	const basePath = control.value.path || ''

	const branchKey = info.key
	for (const propName of Object.keys(info.schema.properties)) {
		if (!info.isExternalTagged && propName === discriminatorKey.value) continue
		const fullPath = basePath ? `${basePath}.${propName}` : propName
		// Include branch alias hint check: e.g. config.attribute_filter.config
		const branchSpecificPath = basePath
			? `${basePath}.${branchKey}.${propName}`
			: `${branchKey}.${propName}`
		const rootDataWithBranch = {
			...(control.value as any).rootData,
			[basePath || 'config']: {
				...((control.value as any).rootData?.[basePath || 'config'] || {}),
				[discriminatorKey.value]: branchKey
			}
		}
		const ctrl = createControlElement(propName, fullPath, formHints.value, rootDataWithBranch)
		// If branchSpecificPath has a hint (e.g. widget: 'attribute-filter'), merge it into ctrl options
		const branchHint = resolveHintForPath(
			branchSpecificPath,
			formHints.value,
			rootDataWithBranch
		)
		if (branchHint) {
			ctrl.options = { ...ctrl.options, ...branchHint }
			if (branchHint.label) ctrl.label = branchHint.label
		}
		subElements.push(ctrl)
	}

	return {
		type: 'VerticalLayout',
		elements: subElements
	} as any
})

const onBranchChange = (newVal: any) => {
	const targetBranch = normalizedBranches.value.find((b) => b.key === newVal)
	if (!targetBranch) return

	if (targetBranch.isNull) {
		handleChange(control.value.path, null)
		return
	}

	if (targetBranch.isExternalTagged && targetBranch.externalPropertyKey) {
		const extKey = targetBranch.externalPropertyKey
		const extPropSchema = (targetBranch.schema.properties as any)[extKey]
		// Determine default inner value (object with defaults or string)
		let defaultVal: any = {}
		if (extPropSchema?.type === 'string') {
			defaultVal = ''
		}
		handleChange(control.value.path, {
			[extKey]: defaultVal
		})
		return
	}

	// Internal tagged
	let nextConfig: any = undefined

	if (newVal === 'dummy') {
		const dummyProps = (targetBranch.schema.properties as any)?.config?.properties
		if (dummyProps?.messages_per_batch !== undefined || dummyProps?.speed !== undefined) {
			nextConfig = {
				messages_per_batch: 100,
				speed: 5
			}
		} else {
			nextConfig = {
				count: 1,
				payload: {
					row: {
						observed_time: null,
						severity: 'info',
						source_ip: null,
						source_port: null,
						body: null,
						attrs: []
					}
				}
			}
		}
	} else if (newVal === 'snmp') {
		nextConfig = { oid: '' }
	} else if (newVal === 'snmp_walk') {
		nextConfig = { oids: [] }
	} else if (newVal === 'prometheus_scrape') {
		nextConfig = { accept_protobuf: false, stream: false }
	} else if (targetBranch.schema.properties?.config) {
		nextConfig = {}
	}

	handleChange(control.value.path, {
		[discriminatorKey.value]: newVal,
		...(nextConfig !== undefined ? { config: nextConfig } : {})
	})
}

const discriminatorLabel = computed(() => {
	return (
		control.value.uischema?.options?.discriminatorLabel ||
		selfHint.value?.discriminatorLabel ||
		control.value.label ||
		'Select Type'
	)
})

const discriminatorPlaceholder = computed(() => {
	return (
		control.value.uischema?.options?.discriminatorPlaceholder ||
		selfHint.value?.discriminatorPlaceholder ||
		'Select option...'
	)
})

// Check if union is simply 'null | single-object' (e.g. authentication, privacy)
const isOptionalObjectUnion = computed(() => {
	if (normalizedBranches.value.length !== 2) return false
	const hasNull = normalizedBranches.value.some((b) => b.isNull)
	const nonNullBranches = normalizedBranches.value.filter((b) => !b.isNull)
	if (!hasNull || nonNullBranches.length !== 1) return false
	const schema = nonNullBranches[0].schema as any
	return schema?.type === 'object' || Boolean(schema?.properties)
})

const singleObjectBranch = computed(() => {
	if (!isOptionalObjectUnion.value) return null
	return normalizedBranches.value.find((b) => !b.isNull) || null
})

const isCheckboxActive = computed(() => {
	const data = control.value.data
	if (data === null || data === undefined) return false
	if (typeof data === 'object') {
		// If object is empty or non-empty, consider active
		return true
	}
	return false
})

const checkboxLabel = computed(() => {
	if (selfHint.value?.label) return selfHint.value.label
	if (control.value.label) return `Use ${control.value.label}`
	return `Use ${formatFieldLabel(leafName.value)}`
})

const onCheckboxToggle = (val: boolean) => {
	if (!val) {
		handleChange(control.value.path, null)
	} else {
		const target = singleObjectBranch.value
		if (target?.isExternalTagged && target.externalPropertyKey) {
			const extKey = target.externalPropertyKey
			const extProp = (target.schema.properties as any)[extKey]
			handleChange(control.value.path, {
				[extKey]: extProp?.type === 'string' ? '' : {}
			})
		} else {
			handleChange(control.value.path, {})
		}
	}
}

// Auto-populate discriminator when branchOptions only has 1 option and current value is empty
watchEffect(() => {
	if (!isOptionalObjectUnion.value) {
		const data = control.value.data
		const currentDisc = data?.[discriminatorKey.value]
		if (
			branchOptions.value.length === 1 &&
			(currentDisc === undefined || currentDisc === null || currentDisc === '')
		) {
			const onlyOption = branchOptions.value[0].value
			onBranchChange(onlyOption)
		}
	}
})
</script>

<template>
	<div
		v-if="control.visible"
		:data-invalid="!!control.errors"
		:class="{ 'has-error': !!control.errors }"
		class="w-full"
	>
		<!-- Custom Slot Fallback -->
		<CustomSlotRenderer v-if="customSlot" :render-fn="customSlot" :slot-props="slotProps" />

		<!-- Union Controls Container -->
		<div v-else class="flex flex-col gap-3 p-3 border border-gray-200 rounded-lg bg-gray-50/50">
			<!-- Mode 1: Checkbox for null | single-object -->
			<template v-if="isOptionalObjectUnion">
				<div class="flex items-center gap-2 py-1">
					<UiCheckbox
						:id="`union-check-${control.path}`"
						:checked="isCheckboxActive"
						:disabled="!control.enabled"
						@update:checked="onCheckboxToggle"
					/>
					<label
						:for="`union-check-${control.path}`"
						class="text-sm font-medium leading-none cursor-pointer select-none text-foreground"
						:class="{ 'opacity-50 cursor-not-allowed': !control.enabled }"
					>
						{{ checkboxLabel }}
					</label>
				</div>

				<!-- Child inputs revealed when checkbox is checked -->
				<div
					v-if="isCheckboxActive && singleObjectBranch && activeBranchUiSchema"
					class="pt-2 flex flex-col gap-3"
				>
					<dispatch-renderer
						:schema="singleObjectBranch.schema"
						:uischema="activeBranchUiSchema"
						:path="control.path"
						:enabled="control.enabled"
						:renderers="control.renderers"
					/>
				</div>
			</template>

			<!-- Mode 2: Multi-branch Union Selector (e.g. protocols, auth basic vs bearer vs none) -->
			<template v-else>
				<!-- Discriminator Selector -->
				<AppInputSelect
					:model-value="currentBranchValue"
					:label="discriminatorLabel"
					:placeholder="discriminatorPlaceholder"
					required
					:options="branchOptions"
					:disabled="!control.enabled"
					@update:model-value="onBranchChange"
				/>

				<!-- Child Fields Dispatcher -->
				<div
					v-if="activeBranch && activeBranchUiSchema && !activeBranchInfo?.isNull"
					class="pt-2 flex flex-col gap-3"
				>
					<dispatch-renderer
						:schema="activeBranch"
						:uischema="activeBranchUiSchema"
						:path="control.path"
						:enabled="control.enabled"
						:renderers="control.renderers"
					/>
				</div>
			</template>

			<AppInputError :error="control.errors" />
		</div>
	</div>
</template>

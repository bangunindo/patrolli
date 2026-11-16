<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed, watch } from 'vue'
import type { FieldDescriptor } from '@@/shared/types/dynamic-form.type'
import AppInputSelect from '@/components/app/input/AppInputSelect.vue'
import AppInputError from '@/components/app/input/AppInputError.vue'
import UiCheckbox from '@/components/ui/checkbox/Checkbox.vue'
import CustomSlotRenderer from './CustomSlotRenderer.vue'
import DynamicFieldRenderer from '../DynamicFieldRenderer.vue'
import { formatFieldLabel, safeClone } from '@/app/services/forms/path-helper'
import { useDynamicField } from '../useDynamicField'

const props = defineProps<{
	descriptor: FieldDescriptor
	path: string
}>()

const { value, setValue, error, hint, label, disabled, leafName, customSlot, slotProps } =
	useDynamicField(props)

const discriminatorKey = computed(() => {
	return hint.value.discriminatorField || props.descriptor.unionDiscriminator || 'type'
})

const isExternalTagged = computed(() => props.descriptor.unionTagMode === 'external')

const rawOptions = computed(() => props.descriptor.options || [])
const unionBranches = computed(() => props.descriptor.unionBranches || {})

const branchOptions = computed(() => {
	const opts = [...rawOptions.value]
	const order = hint.value.optionsOrder
	if (Array.isArray(order) && order.length > 0) {
		opts.sort((a, b) => {
			const idxA = order.indexOf(a.value)
			const idxB = order.indexOf(b.value)
			return (idxA === -1 ? 999 : idxA) - (idxB === -1 ? 999 : idxB)
		})
	}
	return opts
})

const isOptionalObjectUnion = computed(() => {
	if (branchOptions.value.length !== 2) return false
	const hasNone = branchOptions.value.some((o) => o.value === 'none')
	const nonNull = branchOptions.value.filter((o) => o.value !== 'none')
	return hasNone && nonNull.length === 1
})

const singleObjectBranchKey = computed(() => {
	if (!isOptionalObjectUnion.value) return null
	return branchOptions.value.find((o) => o.value !== 'none')?.value || null
})

const currentBranchValue = computed(() => {
	const data = value.value
	if (data === null || data === undefined) {
		if (branchOptions.value.some((o) => o.value === 'none')) {
			return 'none'
		}
	}

	if (data && typeof data === 'object') {
		if (discriminatorKey.value in data && data[discriminatorKey.value] !== undefined) {
			return String(data[discriminatorKey.value])
		}
		if (discriminatorKey.value === 'type' && 'module' in data && data.module !== undefined) {
			return String(data.module)
		}
		if (discriminatorKey.value === 'module' && 'type' in data && data.type !== undefined) {
			return String(data.type)
		}
		for (const opt of branchOptions.value) {
			if (opt.value !== 'none' && opt.value in data) {
				return opt.value
			}
		}
	}

	return hint.value.discriminatorDefault || branchOptions.value[0]?.value || ''
})

const activeBranchFields = computed<FieldDescriptor[]>(() => {
	const branchKey = isOptionalObjectUnion.value
		? singleObjectBranchKey.value
		: currentBranchValue.value
	if (!branchKey || branchKey === 'none') return []
	return unionBranches.value[branchKey] || []
})

const buildBranchChildPath = (child: FieldDescriptor) => {
	return props.path ? `${props.path}.${child.name}` : child.name
}

const buildBranchInitialValue = (branchKey: string): Record<string, any> => {
	const fields = unionBranches.value[branchKey] || []
	const branchDefault = hint.value.branchDefaults?.[branchKey]

	if (isExternalTagged.value) {
		if (branchDefault !== undefined) {
			return { [branchKey]: safeClone(branchDefault) }
		}
		const isPrimitiveBranch =
			fields.length === 1 &&
			fields[0].name === branchKey &&
			fields[0].widget !== 'custom' &&
			!fields[0].children?.length
		return { [branchKey]: isPrimitiveBranch ? '' : {} }
	}

	const configField = fields.find((f) => f.name === 'config')
	const nextConfig =
		branchDefault !== undefined ? safeClone(branchDefault) : configField ? {} : undefined

	return {
		[discriminatorKey.value]: branchKey,
		...(nextConfig !== undefined ? { config: nextConfig } : {})
	}
}

const onBranchChange = (newVal: any) => {
	if (!newVal) return
	if (newVal === 'none') {
		setValue(null)
		return
	}
	setValue(buildBranchInitialValue(newVal))
}

const discriminatorLabel = computed(() => {
	return hint.value.discriminatorLabel || label.value || 'Select Type'
})

const discriminatorPlaceholder = computed(() => {
	return hint.value.discriminatorPlaceholder || 'Select option...'
})

const isCheckboxActive = computed(() => {
	const data = value.value
	if (data === null || data === undefined) return false
	return typeof data === 'object'
})

const checkboxLabel = computed(() => {
	if (hint.value.label) return hint.value.label
	if (label.value) return `Use ${label.value}`
	return `Use ${formatFieldLabel(leafName.value)}`
})

const onCheckboxToggle = (val: boolean) => {
	if (!val) {
		setValue(null)
		return
	}

	const key = singleObjectBranchKey.value
	if (isExternalTagged.value && key) {
		setValue(buildBranchInitialValue(key))
	} else {
		const branchDefault = key ? hint.value.branchDefaults?.[key] : undefined
		setValue(branchDefault !== undefined ? safeClone(branchDefault) : {})
	}
}

watch(
	[() => branchOptions.value, () => value.value],
	([opts, data]) => {
		if (isOptionalObjectUnion.value || opts.length === 0) return

		if (opts.length === 1) {
			const onlyKey = opts[0].value
			const hasExistingBranch = isExternalTagged.value
				? Boolean(data && typeof data === 'object' && onlyKey in data)
				: Boolean(data?.[discriminatorKey.value])
			if (!hasExistingBranch) {
				onBranchChange(onlyKey)
			}
			return
		}

		const hasNoneOption = opts.some((o) => o.value === 'none')
		const isEmptyObj =
			typeof data === 'object' && data !== null && Object.keys(data).length === 0
		const isUninitialized =
			data === undefined || isEmptyObj || (data === null && !hasNoneOption)

		if (!isUninitialized) return

		const targetBranch = hint.value.discriminatorDefault || opts[0]?.value
		if (targetBranch && targetBranch !== 'none') {
			onBranchChange(targetBranch)
		}
	},
	{ immediate: true }
)
</script>

<template>
	<div :data-invalid="!!error" :class="{ 'has-error': !!error }" class="w-full">
		<CustomSlotRenderer v-if="customSlot" :render-fn="customSlot" :slot-props="slotProps" />

		<div v-else class="flex flex-col gap-3 p-3 border border-border rounded-lg bg-muted/30">
			<!-- Mode 1: Checkbox for null | single-object -->
			<template v-if="isOptionalObjectUnion">
				<div class="flex items-center gap-2 py-1">
					<UiCheckbox
						:id="`union-check-${path}`"
						:checked="isCheckboxActive"
						:disabled="disabled"
						@update:checked="onCheckboxToggle"
					/>
					<label
						:for="`union-check-${path}`"
						class="text-sm font-medium leading-none cursor-pointer select-none text-foreground"
						:class="{ 'opacity-50 cursor-not-allowed': disabled }"
					>
						{{ checkboxLabel }}
					</label>
				</div>

				<div
					v-if="isCheckboxActive && activeBranchFields.length > 0"
					class="pt-2 flex flex-col gap-3"
				>
					<DynamicFieldRenderer
						v-for="child in activeBranchFields"
						:key="`${singleObjectBranchKey}-${buildBranchChildPath(child)}`"
						:descriptor="child"
						:path="buildBranchChildPath(child)"
					/>
				</div>
			</template>

			<!-- Mode 2: Multi-branch Union Selector -->
			<template v-else>
				<AppInputSelect
					:model-value="currentBranchValue"
					:label="discriminatorLabel"
					:placeholder="discriminatorPlaceholder"
					required
					:options="branchOptions"
					:disabled="disabled"
					@update:model-value="onBranchChange"
				/>

				<div
					v-if="currentBranchValue !== 'none' && activeBranchFields.length > 0"
					class="pt-2 flex flex-col gap-3"
				>
					<DynamicFieldRenderer
						v-for="child in activeBranchFields"
						:key="`${currentBranchValue}-${buildBranchChildPath(child)}`"
						:descriptor="child"
						:path="buildBranchChildPath(child)"
					/>
				</div>
			</template>

			<AppInputError :error="error" />
		</div>
	</div>
</template>

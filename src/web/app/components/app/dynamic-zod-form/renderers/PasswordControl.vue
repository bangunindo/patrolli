<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { rendererProps, useJsonFormsControl } from '@jsonforms/vue'
import type { ControlElement } from '@jsonforms/core'
import Password from '@/components/app/input/Password.vue'
import UiButton from '@/components/ui/button/Button.vue'
import UiBadge from '@/components/ui/badge/Badge.vue'
import UiIcon from '@/components/ui/icon/Icon.vue'
import { isSensitiveSetObject, isNullableJsonSchema } from '@/app/services/forms/sensitive.utils'

const props = defineProps({
	...rendererProps<ControlElement>()
})

const { control, handleChange } = useJsonFormsControl(props)

// Track previously saved handle from backend if present
const savedSetHandle = ref<string | null>(null)
const mode = ref<'configured' | 'editing' | 'removed'>('editing')

const isOptional = computed(
	() => !control.value.required || isNullableJsonSchema(control.value.schema)
)

watch(
	() => control.value.data,
	(newData) => {
		if (isSensitiveSetObject(newData)) {
			savedSetHandle.value = newData.set
			mode.value = 'configured'
		} else if (typeof newData === 'string') {
			// User is typing or editing a new value; preserve editing mode and savedSetHandle
			mode.value = 'editing'
		} else if (mode.value === 'removed' && (newData === null || newData === undefined)) {
			// Preserve removed state
		} else {
			// External reset or clean entity without configured secret
			savedSetHandle.value = null
			mode.value = 'editing'
		}
	},
	{ immediate: true }
)

const startEditing = () => {
	mode.value = 'editing'
	handleChange(control.value.path, '')
}

const revertToSaved = () => {
	if (savedSetHandle.value) {
		mode.value = 'configured'
		handleChange(control.value.path, { set: savedSetHandle.value })
	}
}

const removeSecret = () => {
	mode.value = 'removed'
	const nullOrUndefined = isNullableJsonSchema(control.value.schema) ? null : undefined
	handleChange(control.value.path, nullOrUndefined)
}

const undoRemove = () => {
	if (savedSetHandle.value) {
		mode.value = 'configured'
		handleChange(control.value.path, { set: savedSetHandle.value })
	} else {
		mode.value = 'editing'
		handleChange(control.value.path, '')
	}
}

const onInput = (val: string | number | null) => {
	handleChange(control.value.path, val === null ? '' : String(val))
}

const stringModelValue = computed(() => {
	const d = control.value.data
	return typeof d === 'string' ? d : ''
})

const passwordRules = computed(() => {
	if (savedSetHandle.value) {
		return ['non-empty-secret']
	}
	return []
})
</script>

<template>
	<div
		v-if="control.visible"
		:data-invalid="!!control.errors"
		:class="{ 'has-error': !!control.errors }"
		class="w-full space-y-1.5"
	>
		<!-- Mode 1: Configured on server -->
		<div v-if="mode === 'configured'" class="w-full space-y-1.5 p-1">
			<label
				v-if="control.label"
				class="text-sm font-semibold text-foreground flex items-center gap-1"
			>
				<span>{{ control.label }}</span>
				<span v-if="control.required" class="text-destructive">*</span>
			</label>

			<div class="flex items-center justify-between gap-2 p-2 rounded-md border bg-muted/30">
				<div class="flex items-center gap-2 text-sm text-muted-foreground">
					<UiBadge variant="secondary" class="flex items-center gap-1 font-normal">
						<UiIcon name="lock" size="14px" class="text-emerald-500" />
						Configured on server
					</UiBadge>
					<span class="text-xs text-muted-foreground hidden sm:inline">
						(Kept unchanged)
					</span>
				</div>

				<div class="flex items-center gap-1">
					<UiButton
						type="button"
						variant="outline"
						size="sm"
						class="h-7 text-xs flex items-center gap-1"
						:disabled="!control.enabled"
						@click="startEditing"
					>
						<UiIcon name="pencil" size="14px" />
						Change
					</UiButton>
					<UiButton
						v-if="isOptional"
						type="button"
						variant="ghost"
						size="sm"
						class="h-7 text-xs text-destructive hover:text-destructive hover:bg-destructive/10 flex items-center gap-1"
						:disabled="!control.enabled"
						title="Remove secret from server"
						@click="removeSecret"
					>
						<UiIcon name="trash-can-outline" size="14px" />
						Remove
					</UiButton>
				</div>
			</div>
		</div>

		<!-- Mode 2: Removed optional secret -->
		<div v-else-if="mode === 'removed'" class="w-full space-y-1.5 p-1">
			<label
				v-if="control.label"
				class="text-sm font-semibold text-foreground flex items-center gap-1"
			>
				<span>{{ control.label }}</span>
			</label>

			<div
				class="flex items-center justify-between gap-2 p-2 rounded-md border border-dashed bg-muted/20"
			>
				<div class="flex items-center gap-2 text-sm text-destructive">
					<UiBadge variant="destructive" class="flex items-center gap-1 font-normal">
						<UiIcon name="close" size="14px" />
						Secret removed
					</UiBadge>
					<span class="text-xs text-muted-foreground hidden sm:inline">
						(Will be cleared on save)
					</span>
				</div>

				<UiButton
					type="button"
					variant="outline"
					size="sm"
					class="h-7 text-xs flex items-center gap-1"
					:disabled="!control.enabled"
					@click="undoRemove"
				>
					<UiIcon name="undo" size="14px" />
					Undo
				</UiButton>
			</div>
		</div>

		<!-- Mode 3: Editing / New secret -->
		<div v-else class="w-full space-y-1">
			<Password
				:model-value="stringModelValue"
				:label="control.label"
				:placeholder="control.uischema?.options?.placeholder || `Enter ${control.label}`"
				:required="control.required || Boolean(savedSetHandle)"
				:rules="passwordRules"
				:error="control.errors"
				:disabled="!control.enabled"
				@update:model-value="onInput"
			/>
			<div
				v-if="savedSetHandle"
				class="flex items-center justify-between px-1 text-xs text-muted-foreground"
			>
				<span class="italic">Changing secret...</span>
				<div class="flex items-center gap-1">
					<UiButton
						type="button"
						variant="ghost"
						size="sm"
						class="h-6 px-1.5 text-xs text-primary hover:underline flex items-center gap-1"
						@click="revertToSaved"
					>
						<UiIcon name="undo" size="13px" />
						Revert
					</UiButton>
					<UiButton
						v-if="isOptional"
						type="button"
						variant="ghost"
						size="sm"
						class="h-6 px-1.5 text-xs text-destructive hover:underline flex items-center gap-1"
						@click="removeSecret"
					>
						<UiIcon name="trash-can-outline" size="13px" />
						Remove
					</UiButton>
				</div>
			</div>
		</div>
	</div>
</template>

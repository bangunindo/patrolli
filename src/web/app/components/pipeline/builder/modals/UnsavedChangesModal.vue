<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed } from 'vue'

const props = withDefaults(
	defineProps<{
		open: boolean
		isSaving?: boolean
	}>(),
	{
		open: false,
		isSaving: false
	}
)

const emit = defineEmits<{
	(e: 'update:open', value: boolean): void
	(e: 'cancel'): void
	(e: 'discard'): void
	(e: 'save'): void
}>()

const isOpen = computed({
	get: () => props.open,
	set: (val: boolean) => emit('update:open', val)
})

function handleCancel() {
	if (props.isSaving) return
	emit('cancel')
}

function handleDiscard() {
	if (props.isSaving) return
	emit('discard')
}

function handleSave() {
	if (props.isSaving) return
	emit('save')
}
</script>

<template>
	<UiDialog v-model:open="isOpen">
		<UiDialogContent class="sm:max-w-md p-6">
			<UiDialogHeader class="flex flex-col items-center text-center sm:text-center">
				<div
					class="p-3 mb-3 bg-amber-50 dark:bg-amber-950/40 border border-amber-200 dark:border-amber-900 rounded-full"
				>
					<UiIcon name="alert" class="w-8 h-8 text-amber-500" />
				</div>
				<UiDialogTitle class="text-lg font-bold text-foreground">
					Unsaved Changes
				</UiDialogTitle>
				<UiDialogDescription class="text-sm text-muted-foreground mt-2 max-w-sm">
					You have unsaved changes in this pipeline builder. Would you like to save your
					changes before leaving?
				</UiDialogDescription>
			</UiDialogHeader>

			<UiDialogFooter
				class="flex flex-col sm:flex-row gap-2 pt-4 border-t border-border sm:justify-between items-center w-full"
			>
				<UiButton
					variant="ghost"
					size="sm"
					class="w-full sm:w-auto"
					:disabled="props.isSaving"
					@click="handleCancel"
				>
					Cancel
				</UiButton>

				<div class="flex flex-col sm:flex-row gap-2 w-full sm:w-auto sm:justify-end">
					<UiButton
						variant="outline"
						size="sm"
						class="w-full sm:w-auto border-destructive/40 text-destructive hover:bg-destructive/10 hover:text-destructive"
						:disabled="props.isSaving"
						@click="handleDiscard"
					>
						Discard & Leave
					</UiButton>

					<UiButton
						size="sm"
						class="w-full sm:w-auto bg-amber-500 hover:bg-amber-600 text-white"
						:disabled="props.isSaving"
						@click="handleSave"
					>
						<UiIcon
							v-if="props.isSaving"
							name="loading"
							class="w-3.5 h-3.5 mr-1.5 animate-spin"
						/>
						<span>{{ props.isSaving ? 'Saving...' : 'Save & Leave' }}</span>
					</UiButton>
				</div>
			</UiDialogFooter>
		</UiDialogContent>
	</UiDialog>
</template>

<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { ref, watch } from 'vue'

const props = defineProps({
	showModal: {
		type: Boolean,
		default: false
	},
	title: {
		type: String,
		default: 'Delete'
	},
	message: {
		type: String,
		default: 'Are you sure want to delete this item?'
	},
	confirmText: {
		type: String,
		default: 'Delete'
	},
	cancelText: {
		type: String,
		default: 'Cancel'
	},
	loading: {
		type: Boolean,
		default: false
	}
})

const emit = defineEmits(['dismiss', 'confirm'])

const isModalShow = ref<boolean>(props.showModal)

watch(
	() => props.showModal,
	(value) => {
		isModalShow.value = value
	}
)
const onOpenChange = (open: boolean) => {
	isModalShow.value = open
	if (!open) {
		emit('dismiss')
	}
}
</script>

<template>
	<UiDialog :open="isModalShow" @update:open="onOpenChange">
		<UiDialogContent class="sm:max-w-md">
			<UiDialogHeader>
				<UiDialogTitle class="text-lg font-semibold">{{ title }}</UiDialogTitle>
			</UiDialogHeader>

			<div class="flex flex-col items-center py-6">
				<div class="text-center whitespace-pre-wrap text-sm text-muted-foreground">
					{{ message }}
				</div>
			</div>

			<UiDialogFooter class="flex flex-row gap-3 justify-end">
				<UiButton variant="outline" size="sm" @click="emit('dismiss')">
					{{ cancelText }}
				</UiButton>
				<UiButton
					variant="destructive"
					size="sm"
					:disabled="loading"
					@click="emit('confirm')"
				>
					<UiIcon v-if="loading" name="loading" class="w-4 h-4 mr-2 animate-spin" />
					{{ confirmText }}
				</UiButton>
			</UiDialogFooter>
		</UiDialogContent>
	</UiDialog>
</template>

<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup>
import { useAppStore } from '@/stores/app.store'

const appStore = useAppStore()
</script>

<template>
	<UiDialog :open="appStore.showFormModal" @update:open="(val) => appStore.updateFormModal(val)">
		<UiDialogContent class="sm:max-w-xl">
			<UiDialogHeader>
				<UiDialogTitle class="text-lg font-semibold">HEADING</UiDialogTitle>
			</UiDialogHeader>

			<div class="flex flex-col gap-2 my-2">Content</div>

			<UiDialogFooter class="text-center sm:justify-center"> Footer </UiDialogFooter>
		</UiDialogContent>
	</UiDialog>
</template>

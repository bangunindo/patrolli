<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup>
import { ref } from 'vue'
import { storeToRefs } from 'pinia'

import { useAppStore } from '@/stores/app.store'
import { useAuthStore } from '@/stores/auth.store'

const appStore = useAppStore()
const authStore = useAuthStore()

const { authenticated } = storeToRefs(authStore)
const loadingProcess = ref(false)

const closeModal = () => {
	appStore.updateConfirmModal(false)
}

const confirmAction = () => {
	loadingProcess.value = true
	authenticated.value = false

	setTimeout(async () => {
		loadingProcess.value = false
		closeModal()
		await navigateTo('/account')
	}, 1000)
}
</script>

<template>
	<UiDialog
		:open="appStore.showConfirmModal"
		@update:open="(val) => appStore.updateConfirmModal(val)"
	>
		<UiDialogContent class="sm:max-w-sm">
			<div class="flex flex-col py-8 items-center">
				<div class="p-4 mb-3 bg-amber-50 dark:bg-amber-950/30 rounded-xl">
					<UiIcon name="alert" class="w-10 h-10 text-amber-500" />
				</div>
				<UiDialogTitle class="text-center text-xl font-bold">Sign Out?</UiDialogTitle>
				<UiDialogDescription class="text-center text-muted-foreground mt-1">
					Are you sure want to quit and finish your session?
				</UiDialogDescription>
			</div>

			<UiDialogFooter class="flex flex-row gap-4 justify-between sm:justify-between">
				<UiButton variant="ghost" size="sm" @click="closeModal"> Cancel </UiButton>
				<UiButton
					size="sm"
					class="bg-amber-500 hover:bg-amber-600 text-white"
					:disabled="loadingProcess"
					@click="confirmAction"
				>
					<UiIcon
						v-if="loadingProcess"
						name="loading"
						class="w-4 h-4 mr-2 animate-spin"
					/>
					Confirm
				</UiButton>
			</UiDialogFooter>
		</UiDialogContent>
	</UiDialog>
</template>

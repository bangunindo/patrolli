<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import 'vue-sonner/style.css'
import { toast } from 'vue-sonner'
import ModalConfirmation from '@/components/modal/ModalConfirmation.vue'
import ModalForm from '@/components/modal/ModalForm.vue'
import AppSplashLoader from '@/components/app/loading/AppSplashLoader.vue'
import { useAppStore } from '@/stores/app.store'
import { storeToRefs } from 'pinia'
import { onMounted, watch } from 'vue'

const appStore = useAppStore()
const { notificationItems, isDarkMode } = storeToRefs(appStore)

watch(
	() => isDarkMode.value,
	(newValue) => {
		setThemeMode(newValue)
	}
)

onMounted(() => {
	setThemeMode(isDarkMode.value)
})

const setThemeMode = (useDarkMode: boolean) => {
	if (import.meta.client) {
		if (useDarkMode) {
			document.body.classList.add('dark')
			document.body.classList.remove('light')
		} else {
			document.body.classList.add('light')
			document.body.classList.remove('dark')
		}
	}
}

watch(
	() => notificationItems.value.length,
	(newLen) => {
		if (newLen > 0) {
			const items = [...notificationItems.value]
			notificationItems.value = []
			for (const item of items) {
				const title = item.title || item.message || 'Notification'
				const description = item.description || (item.title ? item.message : undefined)
				const type = item.type || 'info'
				if (type === 'success') {
					toast.success(title, { description })
				} else if (type === 'error') {
					toast.error(title, { description })
				} else if (type === 'warning') {
					toast.warning(title, { description })
				} else {
					toast.info(title, { description })
				}
			}
		}
	}
)
</script>

<template>
	<ClientOnly>
		<div
			id="app-main"
			:class="[
				'w-full min-h-screen bg-background text-foreground',
				isDarkMode ? 'dark' : 'light'
			]"
		>
			<NuxtLoadingIndicator
				color="hsl(var(--primary))"
				:height="3"
				:duration="2000"
				:throttle="100"
			/>
			<NuxtLayout>
				<NuxtPage />
			</NuxtLayout>

			<div id="app-modal">
				<ModalForm class="transition-all" />
				<ModalConfirmation class="transition-all" />
			</div>

			<UiSonner />
		</div>

		<template #fallback>
			<AppSplashLoader />
		</template>
	</ClientOnly>
</template>

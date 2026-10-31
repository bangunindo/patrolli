<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import AppNavbar from '@/components/app/AppNavbar.vue'
import { useAppStore } from '@/stores/app.store'
import { useAuthStore } from '@/stores/auth.store'
import { ref, watch } from 'vue'

const store = useAppStore()
const keys = ref(''.generateSecureString())

watch(
	() => store.activeClusterURL,
	() => {
		const authStore = useAuthStore()
		const authCookie = useCookie<boolean>('is_authenticated')
		if (authStore.authenticated || authCookie.value) {
			keys.value = ''.generateSecureString()
		}
	}
)
</script>

<template>
	<div
		class="default-layout flex flex-col w-screen h-screen overflow-hidden bg-[#F8FAFC] dark:bg-background text-foreground"
	>
		<header class="w-full shrink-0 z-40">
			<AppNavbar />
		</header>

		<main id="main-content" :key="keys" class="flex-1 w-full overflow-auto">
			<slot />
		</main>
	</div>
</template>

<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import cookieHelper from '@/app/helpers/cookie.helper'
import tokenHelper from '@/app/helpers/token.helper'
import { useAuthStore } from '@/stores/auth.store'
import { onMounted, ref } from 'vue'
import { useRoute } from 'vue-router'

definePageMeta({
	layout: 'auth',
	requiresAuth: false
})

const route = useRoute()
const authStore = useAuthStore()
const loading = ref<boolean>(true)
const errorMessage = ref<string>('')

onMounted(async () => {
	const code = route.query.code as string | undefined
	if (!code) {
		errorMessage.value =
			(route.query.error_description as string) ||
			(route.query.error as string) ||
			'Authorization code not found from Keycloak.'
		loading.value = false
		return
	}

	try {
		// Call server endpoint to exchange code for tokens securely
		const response = await $fetch<{ success: boolean; data?: any }>(
			'/api/auth/keycloak/callback',
			{
				method: 'POST',
				body: { code }
			}
		)

		if (response && response.success && response.data?.access_token) {
			const tokens = response.data
			tokenHelper.setTokens({
				access_token: tokens.access_token,
				refresh_token: tokens.refresh_token,
				id_token: tokens.id_token
			})

			cookieHelper.set('is_authenticated', 'true')
			authStore.authenticated = true
			authStore.authProvider = 'keycloak'

			await navigateTo('/')
		} else {
			throw new Error('Failed to retrieve access token from server')
		}
	} catch (err: any) {
		console.error('Keycloak callback error:', err)
		errorMessage.value =
			err?.data?.error?.detail ||
			err?.data?.error_description ||
			err?.message ||
			'Authentication with Keycloak failed.'
	} finally {
		loading.value = false
	}
})
</script>

<template>
	<div
		class="w-full max-w-[480px] bg-white border border-[#E2E8F0] rounded-2xl sm:rounded-3xl lg:rounded-[24px] p-6 sm:p-8 lg:p-10 shadow-[0px_18px_42px_-12px_rgba(0,0,0,0.35)] backdrop-blur-md transition-all text-center"
	>
		<!-- Header: Brand Logo -->
		<div class="flex flex-col items-center mb-4">
			<div
				class="text-[32px] sm:text-[37.6px] font-bold leading-none tracking-[0.089em] select-none mb-2"
			>
				<span class="text-primary">PATR</span><span class="text-foreground">OLLI</span>
			</div>
		</div>

		<div v-if="loading" class="py-6 flex flex-col items-center gap-4">
			<div
				class="w-10 h-10 border-4 border-primary border-t-transparent rounded-full animate-spin"
			></div>
			<p class="text-base font-semibold text-slate-700">Authenticating with Keycloak...</p>
		</div>

		<div v-else-if="errorMessage" class="py-4 flex flex-col items-center gap-4">
			<div
				class="w-12 h-12 rounded-full bg-red-100 text-red-600 flex items-center justify-center"
			>
				<svg class="w-6 h-6" fill="none" stroke="currentColor" viewBox="0 0 24 24">
					<path
						stroke-linecap="round"
						stroke-linejoin="round"
						stroke-width="2"
						d="M6 18L18 6M6 6l12 12"
					/>
				</svg>
			</div>
			<div>
				<h4 class="font-bold text-lg text-red-600">Login Failed</h4>
				<p class="text-sm text-gray-600 mt-1">{{ errorMessage }}</p>
			</div>
			<UiButton class="w-full mt-2" @click="navigateTo('/account/login')">
				Back to Login
			</UiButton>
		</div>
	</div>
</template>

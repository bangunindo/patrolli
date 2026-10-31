<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import appHelper from '@/app/helpers/app.helper'
import AppInputCluster from '@/components/app/input/AppInputCluster.vue'
import { useAuthStore } from '@/stores/auth.store'
import type { ILoginBody } from '@@/shared/types/auth.type'
import { storeToRefs } from 'pinia'
import { onMounted, reactive, ref } from 'vue'

definePageMeta({
	layout: 'auth',
	requiresAuth: false
})

const authStore = useAuthStore()
const {} = storeToRefs(authStore)

const formLoading = ref<boolean>(false)
const selectedCluster = ref<string | null>(null)

const usernameInput = reactive({
	name: 'b-login-form__email',
	value: '',
	status: null,
	message: '',
	required: true
})

const passwordInput = reactive({
	name: 'b-login-form__password',
	value: '',
	status: null,
	message: '',
	required: true
})
const passwordView = ref<boolean>(true)
const togglePasswordView = () => {
	passwordView.value = !passwordView.value
}

onMounted(() => {
	const defaultCluster = appHelper.getDefaultClusterValue()
	if (defaultCluster) {
		selectedCluster.value = defaultCluster
	}
})

const doLogin = async () => {
	try {
		formLoading.value = true
		const payload: ILoginBody = {
			username: usernameInput.value,
			password: passwordInput.value
		}

		await authStore.login(payload, selectedCluster.value || undefined)
		if (authStore.authenticated) {
			await navigateTo('/')
		}
	} catch (error) {
		console.error(error)
	} finally {
		formLoading.value = false
	}
}

const keycloakLoading = ref<boolean>(false)
const loginWithKeycloak = async () => {
	try {
		keycloakLoading.value = true
		const response = await $fetch<{ success: boolean; data?: { url: string } }>(
			'/api/auth/keycloak/login'
		)
		if (response && response.data?.url) {
			window.location.href = response.data.url
		} else {
			throw new Error('No login URL returned from server')
		}
	} catch (error) {
		console.error(error)
		keycloakLoading.value = false
	}
}
</script>

<template>
	<div
		class="w-full max-w-[480px] bg-white border border-[#E2E8F0] rounded-2xl sm:rounded-3xl lg:rounded-[24px] p-6 sm:p-8 lg:p-10 shadow-[0px_18px_42px_-12px_rgba(0,0,0,0.35)] backdrop-blur-md transition-all duration-200"
	>
		<!-- Header: Brand Logo & Title -->
		<div class="flex flex-col items-center mb-4">
			<div
				class="text-[32px] sm:text-[37.6px] font-bold leading-none tracking-[0.089em] select-none mb-3"
			>
				<span class="text-primary">PATR</span><span class="text-foreground">OLLI</span>
			</div>
			<h1
				class="text-2xl sm:text-[18px] font-bold leading-tight text-[#0F172A] tracking-tight"
			>
				Sign In
			</h1>
		</div>

		<!-- Login Form -->
		<form
			id="kr-login-form"
			method="POST"
			class="flex flex-col gap-4"
			@submit.prevent="doLogin"
		>
			<AppInputCluster
				v-model="selectedCluster"
				label="Cluster"
				label-class="font-bold text-sm text-[#2D3036] mb-1 block"
				placeholder="Cluster 1"
				:clearable="false"
				:required="true"
			/>

			<div class="flex flex-col gap-1.5">
				<label class="font-bold text-sm text-[#2D3036] dark:text-foreground">
					Username <span v-if="usernameInput.required" class="text-rose-500">*</span>
				</label>
				<UiInput
					v-model="usernameInput.value"
					type="text"
					placeholder="Enter your username"
					:name="usernameInput.name"
				/>
				<span v-if="usernameInput.message" class="text-xs text-rose-500">
					{{ usernameInput.message }}
				</span>
			</div>

			<div class="flex flex-col gap-1.5">
				<label class="font-bold text-sm text-[#2D3036] dark:text-foreground">
					Password <span v-if="passwordInput.required" class="text-rose-500">*</span>
				</label>
				<div class="relative w-full">
					<UiInput
						v-model="passwordInput.value"
						:type="passwordView ? 'password' : 'text'"
						placeholder="Your password"
						:name="passwordInput.name"
						class="pr-10"
					/>
					<UiButton
						type="button"
						variant="ghost"
						size="icon"
						class="h-7 w-7 absolute right-1.5 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
						@click="togglePasswordView"
					>
						<UiIcon :name="passwordView ? 'eye-off' : 'eye'" class="w-4 h-4" />
					</UiButton>
				</div>
				<span v-if="passwordInput.message" class="text-xs text-rose-500">
					{{ passwordInput.message }}
				</span>
			</div>

			<UiButton
				id="kr-login-form__submit-btn"
				type="submit"
				class="w-full"
				:disabled="formLoading"
			>
				<UiIcon v-if="formLoading" name="loading" class="w-4 h-4 mr-2 animate-spin" />
				Sign in
			</UiButton>

			<!-- Divider: Or continue with -->
			<div class="relative flex items-center justify-center my-1">
				<div class="border-t border-[#E2E8F0] dark:border-border w-full absolute"></div>
				<span
					class="bg-white dark:bg-card px-3 text-xs text-[#64748B] dark:text-muted-foreground font-normal relative z-10 select-none"
				>
					Or continue with
				</span>
			</div>

			<!-- SSO: Keycloak Button -->
			<UiButton
				id="kr-login-form__keycloak-btn"
				type="button"
				variant="outline"
				class="w-full bg-background"
				:disabled="keycloakLoading"
				@click="loginWithKeycloak"
			>
				<UiIcon v-if="keycloakLoading" name="loading" class="w-4 h-4 mr-2 animate-spin" />
				Sign in with Keycloak
			</UiButton>
		</form>
	</div>
</template>

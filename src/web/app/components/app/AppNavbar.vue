<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import appHelper from '@/app/helpers/app.helper'
import tokenHelper from '@/app/helpers/token.helper'
import AppInputCluster from '@/components/app/input/AppInputCluster.vue'
import { useAppStore } from '@/stores/app.store'
import { useAuthStore } from '@/stores/auth.store'
import { usePipelineBuildStore } from '@/stores/pipeline.builder.store'
import appConstant from '@@/shared/constants/app.constant'
import { storeToRefs } from 'pinia'
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { useRoute } from 'vue-router'

const route = useRoute()
const appStore = useAppStore()
const authStore = useAuthStore()
const pipelineBuildStore = usePipelineBuildStore()
const { isDarkMode, activeClusterURL } = storeToRefs(appStore)
const { isClusterReadonly } = storeToRefs(authStore)

const isMobileMenuOpen = ref(false)

// Local and Keycloak access tokens both carry preferred_username. The token
// cookie is only readable in the browser, so this is filled in onMounted.
const username = ref('')
const userInitials = computed(() => username.value.slice(0, 2).toUpperCase())

const menuList = computed(() => {
	return appConstant.menuNav
})

const activePath = computed(() => {
	const path = route.path.split('/').filter((item) => item != '')
	return path[0] ?? 'dashboard'
})

const isSingleCluster = computed(() => appHelper.getBaseURLs().length <= 1)

const isPipelineBuilderRoute = computed(() => /^\/pipeline\/[^/]+\/builder(\/|$)/.test(route.path))

const resolvePipelineBuilderCluster = (): string | null => {
	const queryCluster = route.query?.cluster_url
		? String(route.query.cluster_url)
		: pipelineBuildStore.clusterUrl
	if (queryCluster) {
		return appHelper.getClusterValue(queryCluster) || queryCluster
	}
	return appHelper.getDefaultClusterValue() || null
}

const validateActiveCluster = () => {
	const available = appHelper.getBaseURLs()
	if (available.length === 1 && available[0]?.value) {
		activeClusterURL.value = available[0].value
	} else if (available.length > 1) {
		const validValues = available.map((c) => c.value)
		if (isPipelineBuilderRoute.value) {
			activeClusterURL.value = resolvePipelineBuilderCluster()
		} else if (activeClusterURL.value && !validValues.includes(activeClusterURL.value)) {
			activeClusterURL.value = null
		}
	} else {
		activeClusterURL.value = null
	}
}

watch(
	() => appHelper.getBaseURLs(),
	() => {
		validateActiveCluster()
	},
	{ immediate: true, deep: true }
)

watch(
	() => route.path,
	(newPath, oldPath) => {
		isMobileMenuOpen.value = false
		if (isClusterReadonly.value) return

		const isEnteringBuilder = /^\/pipeline\/[^/]+\/builder(\/|$)/.test(newPath)
		const isLeavingBuilder = oldPath
			? /^\/pipeline\/[^/]+\/builder(\/|$)/.test(oldPath) && !isEnteringBuilder
			: false

		if (isEnteringBuilder) {
			activeClusterURL.value = resolvePipelineBuilderCluster()
		} else if (isLeavingBuilder) {
			if (!isSingleCluster.value) {
				activeClusterURL.value = null
			}
		} else {
			validateActiveCluster()
		}
	},
	{ immediate: true }
)

watch(
	() => [route.query?.cluster_url, pipelineBuildStore.clusterUrl],
	() => {
		if (isPipelineBuilderRoute.value) {
			activeClusterURL.value = resolvePipelineBuilderCluster()
		}
	}
)

onMounted(() => {
	username.value = tokenHelper.decodeToken().preferred_username ?? ''
	validateActiveCluster()
	const handleResize = () => {
		if (window.innerWidth >= 768 && isMobileMenuOpen.value) {
			isMobileMenuOpen.value = false
		}
	}
	window.addEventListener('resize', handleResize)
	onUnmounted(() => window.removeEventListener('resize', handleResize))
})
</script>

<template>
	<div class="w-full px-3 pt-3">
		<nav
			class="w-full h-14 px-4 md:px-6 flex items-center justify-between border-b border-[#E5E7EB] dark:border-border bg-white dark:bg-card shadow-[0_2px_8px_rgba(0,0,0,0.04)]"
		>
			<!-- Left: Logo & Brand -->
			<div class="flex items-center gap-6 lg:gap-8">
				<NuxtLink to="/configuration/host" class="flex items-center select-none">
					<div class="text-[28px] font-bold leading-none tracking-[0.089em]">
						<span class="text-primary">PATR</span
						><span class="text-foreground">OLLI</span>
					</div>
				</NuxtLink>

				<!-- Center: Navigation Menu -->
				<div class="hidden md:flex items-center gap-2">
					<template v-for="menu in menuList" :key="menu.key">
						<RouterLink
							:to="menu.to"
							class="h-9 px-3 text-sm font-bold transition-all flex items-center justify-center select-none"
							:class="{
								'border border-primary text-primary bg-accent dark:bg-primary/20 rounded-[10px]':
									activePath === menu.key,
								'text-muted-foreground hover:text-foreground hover:bg-accent/60 rounded-[4px]':
									activePath !== menu.key
							}"
						>
							{{ menu.label }}
						</RouterLink>
					</template>
				</div>
			</div>

			<!-- Right: Controls & User Profile -->
			<div class="flex items-center gap-3">
				<!-- Figma-style Dark mode pill switch (w-10 h-5) -->
				<div class="flex items-center">
					<button
						type="button"
						role="switch"
						:aria-checked="isDarkMode"
						class="relative inline-flex h-5 w-10 shrink-0 cursor-pointer items-center rounded-full transition-colors duration-200 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-2"
						:class="isDarkMode ? 'bg-primary' : 'bg-slate-300 dark:bg-slate-700'"
						:title="isDarkMode ? 'Switch to light mode' : 'Switch to dark mode'"
						@click="isDarkMode = !isDarkMode"
					>
						<span
							class="pointer-events-none inline-block h-4 w-4 transform rounded-full bg-white shadow-[0_2px_4px_rgba(0,35,11,0.2)] transition-transform duration-200"
							:class="isDarkMode ? 'translate-x-5' : 'translate-x-0.5'"
						/>
					</button>
				</div>

				<!-- Desktop Cluster Selector -->
				<div class="hidden md:flex items-center pl-1">
					<AppInputCluster
						v-model="activeClusterURL"
						class="w-[180px]"
						:disabled="isClusterReadonly || isSingleCluster || isPipelineBuilderRoute"
						:clearable="
							!isClusterReadonly && !isSingleCluster && !isPipelineBuilderRoute
						"
					/>
				</div>

				<!-- Desktop User Profile Dropdown -->
				<div class="hidden md:flex items-center pl-1">
					<UiDropdownMenu>
						<UiDropdownMenuTrigger>
							<div
								class="flex flex-row items-center cursor-pointer hover:opacity-90 transition-opacity"
							>
								<UiAvatar
									class="h-7 w-7 bg-primary text-primary-foreground ring-2 ring-background text-[12px] shadow-xs cursor-pointer"
								>
									<UiAvatarFallback
										class="bg-primary text-primary-foreground text-[12px] font-semibold"
									>
										{{ userInitials }}
									</UiAvatarFallback>
								</UiAvatar>
							</div>
						</UiDropdownMenuTrigger>

						<UiDropdownMenuContent align="end" class="w-56 p-2">
							<div class="flex items-center gap-2 p-2">
								<UiAvatar size="md" class="bg-primary text-primary-foreground">
									<UiAvatarFallback>{{ userInitials }}</UiAvatarFallback>
								</UiAvatar>
								<span class="text-sm font-semibold text-foreground">{{
									username
								}}</span>
							</div>

							<UiDropdownMenuSeparator />

							<UiDropdownMenuItem as-child class="cursor-pointer">
								<a :href="appConstant.sourceUrl" target="_blank" rel="noopener">
									<UiIcon name="code-tags" size="16px" class="mr-2" />
									<span>Source Code</span>
								</a>
							</UiDropdownMenuItem>

							<UiDropdownMenuItem
								class="text-destructive focus:bg-destructive/10 focus:text-destructive cursor-pointer"
								@click="authStore.logout()"
							>
								<UiIcon name="logout" size="16px" class="mr-2" />
								<span>Log Out</span>
							</UiDropdownMenuItem>
						</UiDropdownMenuContent>
					</UiDropdownMenu>
				</div>

				<!-- Mobile Hamburger Button -->
				<div class="flex md:hidden items-center">
					<UiButton
						variant="outline"
						size="icon"
						class="h-8 w-8"
						@click="isMobileMenuOpen = !isMobileMenuOpen"
					>
						<UiIcon :name="isMobileMenuOpen ? 'close' : 'menu'" size="18px" />
					</UiButton>
				</div>
			</div>
		</nav>

		<!-- Mobile Menu Panel -->
		<div
			v-if="isMobileMenuOpen"
			class="md:hidden p-4 border-b border-border bg-card shadow-md flex flex-col gap-3 transition-all"
		>
			<!-- Cluster Selector in Mobile Menu -->
			<div class="flex flex-col gap-1.5">
				<span class="text-xs font-medium text-muted-foreground uppercase tracking-wider">
					Active Cluster
				</span>
				<AppInputCluster
					v-model="activeClusterURL"
					class="w-full"
					:disabled="isClusterReadonly || isSingleCluster || isPipelineBuilderRoute"
					:clearable="!isClusterReadonly && !isSingleCluster && !isPipelineBuilderRoute"
				/>
			</div>

			<div class="h-px bg-border w-full" />

			<!-- Navigation Links -->
			<div class="flex flex-col gap-1">
				<span class="text-xs font-medium text-muted-foreground uppercase tracking-wider">
					Menu
				</span>
				<template v-for="menu in menuList" :key="menu.key">
					<RouterLink
						:to="menu.to"
						class="px-3 py-2 rounded-[10px] font-semibold text-sm transition-all flex items-center justify-between border"
						:class="{
							'bg-accent/80 border-primary text-primary font-bold':
								activePath === menu.key,
							'border-transparent text-muted-foreground hover:text-foreground hover:bg-accent':
								activePath !== menu.key
						}"
						@click="isMobileMenuOpen = false"
					>
						<span>{{ menu.label }}</span>
						<UiIcon v-if="activePath === menu.key" name="chevron-right" size="16px" />
					</RouterLink>
				</template>
				<a
					:href="appConstant.sourceUrl"
					target="_blank"
					rel="noopener"
					class="px-3 py-2 rounded-[10px] font-semibold text-sm transition-all flex items-center justify-between border border-transparent text-muted-foreground hover:text-foreground hover:bg-accent"
				>
					<span>Source Code</span>
					<UiIcon name="code-tags" size="16px" />
				</a>
			</div>

			<div class="h-px bg-border w-full" />

			<!-- User & Logout Action in Mobile Menu -->
			<div class="flex flex-col gap-2 pt-1">
				<div class="flex items-center gap-2 px-1">
					<UiAvatar size="sm" class="bg-primary text-primary-foreground">
						<UiAvatarFallback>{{ userInitials }}</UiAvatarFallback>
					</UiAvatar>
					<span class="text-xs font-semibold text-foreground">{{ username }}</span>
				</div>
				<UiButton
					variant="outline"
					size="sm"
					class="w-full justify-center text-destructive border-destructive/20 hover:bg-destructive/10 hover:text-destructive"
					@click="authStore.logout()"
				>
					<UiIcon name="logout" size="16px" class="mr-2" />
					Log Out
				</UiButton>
			</div>
		</div>
	</div>
</template>

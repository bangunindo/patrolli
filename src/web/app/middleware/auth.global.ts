// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { useAuthStore } from '@/stores/auth.store'

export default defineNuxtRouteMiddleware((to) => {
	const authStore = useAuthStore()
	const authCookie = useCookie<boolean>('is_authenticated')
	const isAuthenticated = Boolean(authCookie.value || authStore.authenticated)
	const isAuthRequired = to.meta.requiresAuth !== false

	if (isAuthRequired && !isAuthenticated) {
		console.log('>>> [DEBUG auth.global.ts] redirecting to /account/login')
		return navigateTo('/account/login')
	}

	if (!isAuthRequired && isAuthenticated && to.path.startsWith('/account')) {
		console.log('>>> [DEBUG auth.global.ts] redirecting to /configuration/host')
		return navigateTo('/configuration/host')
	}
})

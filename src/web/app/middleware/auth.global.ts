// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { useAuthStore } from '@/stores/auth.store'

export default defineNuxtRouteMiddleware((to) => {
	const authStore = useAuthStore()
	const isAuthenticated = authStore.hasActiveSession()
	const isAuthRequired = to.meta.requiresAuth !== false

	if (isAuthRequired && !isAuthenticated) {
		return navigateTo('/account/login')
	}

	if (!isAuthRequired && isAuthenticated && to.path.startsWith('/account')) {
		return navigateTo('/configuration/host')
	}
})

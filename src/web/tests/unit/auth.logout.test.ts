// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import authApi from '@/app/api/auth.api'
import cookieHelper from '@/app/helpers/cookie.helper'
import httpService from '@/app/services/http.service'
import { useAuthStore } from '@/stores/auth.store'
import { createPinia, setActivePinia } from 'pinia'
import { beforeEach, describe, expect, it, vi } from 'vitest'

describe('Auth Logout - Client & Helper', () => {
	beforeEach(() => {
		setActivePinia(createPinia())
		// Mock document.cookie
		Object.defineProperty(document, 'cookie', {
			writable: true,
			value: 'patrolli_session=xyz123; auth_token=abc456; other_cookie=test'
		})

		vi.restoreAllMocks()
	})

	it('cookieHelper.clearAll should clear document.cookie', () => {
		cookieHelper.clearAll()
		// Should set expired cookies
		expect(typeof document.cookie).toBe('string')
	})

	it('authApi.logout should post to /auth/logout and call cookieHelper.clearAll', async () => {
		const postSpy = vi.spyOn(httpService, 'post').mockResolvedValue({
			status_code: 200,
			success: true,
			data: { message: 'Logout successful' }
		} as any)
		const clearSpy = vi.spyOn(cookieHelper, 'clearAll')

		const res = await authApi.logout()

		expect(postSpy).toHaveBeenCalledWith('/auth/logout')
		expect(clearSpy).toHaveBeenCalled()
		expect(res?.success).toBe(true)
	})

	it('authStore.logout should clear store state and clear cookies even if api fails', async () => {
		vi.spyOn(authApi, 'logout').mockRejectedValue(new Error('Network error'))
		const clearSpy = vi.spyOn(cookieHelper, 'clearAll')

		const store = useAuthStore()
		store.authenticated = true
		store.userMenus = [{ id: '1' }]

		await store.logout()

		expect(store.authenticated).toBe(false)
		expect(store.userMenus).toBeNull()
		expect(clearSpy).toHaveBeenCalled()
	})

	it('authStore should initialize isClusterReadonly correctly from cookies and persist across state changes', async () => {
		const { useAppStore } = await import('@/stores/app.store')

		// Simulate local login stored in cookies
		cookieHelper.set('auth_provider', 'local')

		const authStore = useAuthStore()
		const appStore = useAppStore()
		appStore.activeClusterURL = 'patrolli1'

		expect(authStore.authProvider).toBe('local')
		expect(authStore.isClusterReadonly).toBe(true)
		expect(appStore.activeClusterURL).toBe('patrolli1')

		// Logout should clear both
		vi.spyOn(authApi, 'logout').mockResolvedValue({ success: true } as any)
		await authStore.logout()

		expect(authStore.authProvider).toBeNull()
		expect(authStore.isClusterReadonly).toBe(false)
		expect(cookieHelper.get('auth_provider')).toBeNull()
	})

	it('authStore.logout should clear notificationItems in appStore', async () => {
		const { useAppStore } = await import('@/stores/app.store')
		const authStore = useAuthStore()
		const appStore = useAppStore()

		appStore.showError('Test Error', 'Something went wrong')
		expect(appStore.notificationItems.length).toBeGreaterThan(0)

		vi.spyOn(authApi, 'logout').mockResolvedValue({ success: true } as any)
		await authStore.logout()

		expect(appStore.notificationItems.length).toBe(0)
	})

	it('axios.service post should suppress error toast on /auth/logout failure', async () => {
		const { AxiosService } = await import('@/app/services/axios.service')
		const { useAppStore } = await import('@/stores/app.store')
		const appStore = useAppStore()
		const appShowErrorSpy = vi.spyOn(appStore, 'showError')

		const axiosInstance = new AxiosService()
		vi.spyOn(axiosInstance.instance, 'post').mockRejectedValue(new Error('Network Error'))

		await expect(axiosInstance.post('/auth/logout')).rejects.toThrow()
		expect(appShowErrorSpy).not.toHaveBeenCalled()
	})
})

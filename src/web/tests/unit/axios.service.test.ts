// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { handleErrorMessage, showSuccessMessage } from '@/app/services/axios.service'
import { useAppStore } from '@/stores/app.store'
import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

describe('axios.service notification debounce with VueUse', () => {
	beforeEach(() => {
		vi.useFakeTimers()
		setActivePinia(createPinia())
	})

	afterEach(() => {
		vi.restoreAllMocks()
	})

	it('should debounce rapid error messages using useDebounceFn', async () => {
		const appStore = useAppStore()
		const error = {
			message: 'Request failed with status code 500',
			data: {
				error: {
					title: 'Internal Server Error',
					detail: 'Cluster unavailable'
				}
			}
		}

		// Call 3 times rapidly
		handleErrorMessage(error)
		handleErrorMessage(error)
		handleErrorMessage(error)

		// Before timer passes, debounced function hasn't executed
		expect(appStore.notificationItems).toHaveLength(0)

		// Advance timer past debounce window (500ms)
		await vi.advanceTimersByTimeAsync(500)

		expect(appStore.notificationItems).toHaveLength(1)
		expect(appStore.notificationItems[0].title).toBe('Internal Server Error')
		expect(appStore.notificationItems[0].description).toBe('Cluster unavailable')
	})

	it('should debounce rapid success messages using useDebounceFn', async () => {
		const appStore = useAppStore()
		const mockResponse = {
			data: { success: true },
			status: 200,
			statusText: 'OK',
			headers: {},
			config: {
				method: 'post',
				url: '/api/items'
			} as any
		}

		// Call 3 times rapidly
		showSuccessMessage(mockResponse)
		showSuccessMessage(mockResponse)
		showSuccessMessage(mockResponse)

		// Advance timer
		await vi.advanceTimersByTimeAsync(500)

		expect(appStore.notificationItems).toHaveLength(1)
		expect(appStore.notificationItems[0].title).toBe('Success Add Data')
	})
})

describe('axios.service Bearer token and refresh token handling', () => {
	beforeEach(() => {
		setActivePinia(createPinia())
		Object.defineProperty(document, 'cookie', {
			writable: true,
			value: ''
		})
	})

	afterEach(() => {
		vi.restoreAllMocks()
		document.cookie = ''
	})

	it('should rely on HttpOnly cookies (withCredentials) without injecting Bearer token from document.cookie', async () => {
		document.cookie = 'access_token=test-access-token-123; path=/'
		const { AxiosService } = await import('@/app/services/axios.service')
		const service = new AxiosService()

		expect(service.instance.defaults.withCredentials).toBe(true)

		const requestInterceptor = (service.instance.interceptors.request as any).handlers[0]
			.fulfilled
		const modifiedConfig = await requestInterceptor({
			url: '/api/items',
			headers: {}
		})

		expect(modifiedConfig.headers.Authorization).toBeUndefined()
	})

	it('should attach comma-separated x-cluster-url header when multiple clusters are provided to AxiosService', async () => {
		const { AxiosService } = await import('@/app/services/axios.service')
		const clusters = ['cluster-01', 'cluster-02']
		const service = new AxiosService(clusters.join(','))

		const requestInterceptor = (service.instance.interceptors.request as any).handlers[0]
			.fulfilled
		const modifiedConfig = await requestInterceptor({
			url: '/api/users',
			headers: {}
		})

		expect(modifiedConfig.headers['x-cluster-url']).toBe('cluster-01,cluster-02')
	})

	it('should perform pre-flight token refresh in request interceptor when access_token_exp is within 30s leeway and deduplicate concurrent requests', async () => {
		const { AxiosService } = await import('@/app/services/axios.service')
		const { useAuthStore } = await import('@/stores/auth.store')
		const authApi = (await import('@/app/api/auth.api')).default

		const service = new AxiosService()
		const authStore = useAuthStore()
		authStore.authenticated = true
		const soonExp = Date.now() + 10_000
		document.cookie = `is_authenticated=true; access_token_exp=${soonExp}`

		const refreshApiSpy = vi.spyOn(authApi, 'refreshToken').mockImplementation(async () => {
			const nextExp = Date.now() + 300_000
			document.cookie = `is_authenticated=true; access_token_exp=${nextExp}`
			return {
				status_code: 200,
				success: true,
				data: {
					expires_in: 300,
					user: { preferred_username: 'alice', username: 'alice' }
				}
			}
		})

		const requestInterceptor = (service.instance.interceptors.request as any).handlers[0]
			.fulfilled

		await Promise.all([
			requestInterceptor({ url: '/api/hosts', headers: {} }),
			requestInterceptor({ url: '/api/pipelines', headers: {} }),
			requestInterceptor({ url: '/api/users', headers: {} })
		])

		expect(refreshApiSpy).toHaveBeenCalledTimes(1)
		expect(authStore.isAccessTokenExpiringSoon()).toBe(false)
	})

	it('should skip pre-flight refresh for auth lifecycle paths even when token is expired', async () => {
		const { AxiosService } = await import('@/app/services/axios.service')
		const { useAuthStore } = await import('@/stores/auth.store')

		const service = new AxiosService()
		const authStore = useAuthStore()
		authStore.authenticated = true
		document.cookie = `is_authenticated=true; access_token_exp=${Date.now() - 5000}`

		const ensureSpy = vi.spyOn(authStore, 'ensureValidAccessToken')
		const requestInterceptor = (service.instance.interceptors.request as any).handlers[0]
			.fulfilled

		await requestInterceptor({ url: '/api/auth/refresh', headers: {} })
		await requestInterceptor({ url: '/api/auth/me', headers: {} })
		await requestInterceptor({ url: '/auth/login', headers: {} })
		await requestInterceptor({ url: '/auth/logout', headers: {} })

		expect(ensureSpy).not.toHaveBeenCalled()
	})

	it('should coordinate via navigator.locks and skip HTTP refresh if another tab already updated access_token_exp', async () => {
		const { useAuthStore, AUTH_REFRESH_LOCK_NAME } = await import('@/stores/auth.store')
		const authApi = (await import('@/app/api/auth.api')).default

		const authStore = useAuthStore()
		authStore.authenticated = true
		document.cookie = `is_authenticated=true; access_token_exp=${Date.now() - 1000}`

		const refreshApiSpy = vi.spyOn(authApi, 'refreshToken')

		const mockLockRequest = vi.fn(async (_name: string, callback: () => Promise<any>) => {
			// Simulate another tab completing refresh while this tab waited for the Web Lock
			document.cookie = `is_authenticated=true; access_token_exp=${Date.now() + 300_000}`
			return callback()
		})
		Object.defineProperty(navigator, 'locks', {
			configurable: true,
			value: { request: mockLockRequest }
		})

		try {
			const ok = await authStore.ensureValidAccessToken()
			expect(ok).toBe(true)
			expect(mockLockRequest).toHaveBeenCalledWith(
				AUTH_REFRESH_LOCK_NAME,
				expect.any(Function)
			)
			expect(refreshApiSpy).not.toHaveBeenCalled()
		} finally {
			Object.defineProperty(navigator, 'locks', {
				configurable: true,
				value: undefined
			})
		}
	})
})

describe('httpService Multi-Cluster Routing Integration', () => {
	beforeEach(() => {
		setActivePinia(createPinia())
	})

	it('should join multiple clusterURLs with comma when sending request', async () => {
		const { httpService } = await import('@/app/services/http.service')
		const { createHTTPInstance } = await import('@/app/services/axios.service')

		const mockPost = vi.fn().mockResolvedValue({ data: { success: true } })
		const instance = createHTTPInstance('cluster-01,cluster-02')
		vi.spyOn(instance, 'post').mockImplementation(mockPost)

		await httpService.post('/api/users', { username: 'test' }, undefined, [
			'cluster-01',
			'cluster-02'
		])

		expect(mockPost).toHaveBeenCalledWith('/api/users', { username: 'test' }, undefined)
	})
})

describe('axios.service suppressErrorStatuses', () => {
	beforeEach(() => {
		vi.useFakeTimers()
		setActivePinia(createPinia())
	})

	afterEach(() => {
		vi.useRealTimers()
		vi.restoreAllMocks()
	})

	it('suppresses toast notifications for configured statuses in put()', async () => {
		const { AxiosService } = await import('@/app/services/axios.service')
		const appStore = useAppStore()
		const service = new AxiosService()

		vi.spyOn(service.instance, 'put').mockRejectedValue({
			status: 409,
			message: 'Conflict',
			data: { error: { title: 'Subset in use', detail: 'Still in use' } }
		})

		await expect(
			service.put(
				'/api/pipelines/1',
				{},
				{
					suppressErrorStatuses: [409]
				}
			)
		).rejects.toMatchObject({ status: 409 })

		await vi.advanceTimersByTimeAsync(500)
		expect(appStore.notificationItems).toHaveLength(0)

		vi.spyOn(service.instance, 'put').mockRejectedValueOnce({
			status: 500,
			message: 'Server Error',
			data: { error: { title: 'Internal Error', detail: 'Boom' } }
		})

		await expect(
			service.put(
				'/api/pipelines/1',
				{},
				{
					suppressErrorStatuses: [409]
				}
			)
		).rejects.toMatchObject({ status: 500 })

		await vi.advanceTimersByTimeAsync(500)
		expect(appStore.notificationItems).toHaveLength(1)
		expect(appStore.notificationItems[0].title).toBe('Internal Error')
	})

	it('should NOT call logout when auth lifecycle endpoints return 401', async () => {
		const { AxiosService } = await import('@/app/services/axios.service')
		const { useAuthStore } = await import('@/stores/auth.store')
		const service = new AxiosService()
		const authStore = useAuthStore()
		const logoutSpy = vi.spyOn(authStore, 'logout').mockResolvedValue()

		const errorInterceptor = (service.instance.interceptors.response as any).handlers[0]
			.rejected

		for (const url of [
			'/auth/login',
			'/auth/logout',
			'/api/auth/logout',
			'/api/auth/me',
			'/api/auth/refresh'
		]) {
			await expect(
				errorInterceptor({
					isAxiosError: true,
					config: { url, headers: {} },
					response: { status: 401, data: { message: 'Unauthorized' } }
				})
			).rejects.toMatchObject({ status: 401 })
		}

		expect(logoutSpy).not.toHaveBeenCalled()
	})

	it('should attempt token refresh and retry once when /auth/change-password returns 401', async () => {
		const { AxiosService } = await import('@/app/services/axios.service')
		const { useAuthStore } = await import('@/stores/auth.store')
		const service = new AxiosService()
		const authStore = useAuthStore()
		const refreshSpy = vi.spyOn(authStore, 'refreshAccessToken').mockResolvedValue(true)
		const logoutSpy = vi.spyOn(authStore, 'logout').mockResolvedValue()
		const retryRequestSpy = vi
			.spyOn(service, 'instance' as any)
			.mockResolvedValue({ status: 200, data: { success: true } })

		const errorInterceptor = (service.instance.interceptors.response as any).handlers[0]
			.rejected

		const originalConfig: any = { url: '/auth/change-password', headers: {} }
		const res = await errorInterceptor({
			isAxiosError: true,
			config: originalConfig,
			response: { status: 401, data: { message: 'Unauthorized' } }
		})

		expect(refreshSpy).toHaveBeenCalledTimes(1)
		expect(logoutSpy).not.toHaveBeenCalled()
		expect(originalConfig._retry).toBe(true)
		expect(retryRequestSpy).toHaveBeenCalledWith(originalConfig)
		expect(res).toEqual({ status: 200, data: { success: true } })
	})

	it('rethrows errors in delete() when rethrowErrors is true and returns undefined by default', async () => {
		const { AxiosService } = await import('@/app/services/axios.service')
		const appStore = useAppStore()
		const service = new AxiosService()

		vi.spyOn(service.instance, 'delete').mockRejectedValue({
			status: 400,
			message: 'Bad Request',
			data: { error: { title: 'Delete Failed', detail: 'Cannot delete item' } }
		})

		const defaultResult = await service.delete('/api/items/1')
		expect(defaultResult).toBeUndefined()

		await vi.advanceTimersByTimeAsync(500)
		expect(appStore.notificationItems).toHaveLength(1)
		expect(appStore.notificationItems[0].title).toBe('Delete Failed')

		await expect(
			service.delete('/api/items/1', {
				rethrowErrors: true,
				suppressErrorStatuses: [400]
			})
		).rejects.toMatchObject({ status: 400 })
	})

	it('handles error toast and rethrows errors in patch()', async () => {
		const { AxiosService } = await import('@/app/services/axios.service')
		const appStore = useAppStore()
		const service = new AxiosService()

		vi.spyOn(service.instance, 'patch').mockRejectedValueOnce({
			status: 409,
			message: 'Conflict',
			data: { error: { title: 'Patch Conflict', detail: 'Duplicate username' } }
		})

		await expect(
			service.patch('/api/users/1', { username: 'dup' }, { suppressErrorStatuses: [409] })
		).rejects.toMatchObject({ status: 409 })

		await vi.advanceTimersByTimeAsync(500)
		expect(appStore.notificationItems).toHaveLength(0)

		vi.spyOn(service.instance, 'patch').mockRejectedValueOnce({
			status: 500,
			message: 'Server Error',
			data: { error: { title: 'Patch Failed', detail: 'Unexpected error' } }
		})

		await expect(service.patch('/api/users/1', { username: 'dup' })).rejects.toMatchObject({
			status: 500
		})

		await vi.advanceTimersByTimeAsync(500)
		expect(appStore.notificationItems).toHaveLength(1)
		expect(appStore.notificationItems[0].title).toBe('Patch Failed')
	})
})

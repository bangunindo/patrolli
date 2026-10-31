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

	it('should attach Bearer token to request headers for non-auth endpoints when token exists', async () => {
		document.cookie = 'access_token=test-access-token-123; path=/'
		const { AxiosService } = await import('@/app/services/axios.service')
		const service = new AxiosService()

		vi.spyOn(service.instance, 'request').mockImplementation((_config: any) => {
			return Promise.resolve({ data: { success: true }, status: 200 } as any)
		})

		// Execute request interceptors manually on test config
		const requestInterceptor = (service.instance.interceptors.request as any).handlers[0]
			.fulfilled
		const modifiedConfig = requestInterceptor({
			url: '/api/items',
			headers: {}
		})

		expect(modifiedConfig.headers.Authorization).toBe('Bearer test-access-token-123')
	})

	it('should NOT attach Bearer token for /auth endpoints even if token exists', async () => {
		document.cookie = 'access_token=test-access-token-123; path=/'
		const { AxiosService } = await import('@/app/services/axios.service')
		const service = new AxiosService()

		const requestInterceptor = (service.instance.interceptors.request as any).handlers[0]
			.fulfilled
		const modifiedConfig = requestInterceptor({
			url: '/auth/login',
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
		const modifiedConfig = requestInterceptor({
			url: '/api/users',
			headers: {}
		})

		expect(modifiedConfig.headers['x-cluster-url']).toBe('cluster-01,cluster-02')
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

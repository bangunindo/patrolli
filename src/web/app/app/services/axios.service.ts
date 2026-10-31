// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import axios, {
	type AxiosInstance,
	type AxiosRequestConfig,
	type AxiosResponse,
	type InternalAxiosRequestConfig
} from 'axios'

import { useAppStore } from '@/stores/app.store'
import { useAuthStore } from '@/stores/auth.store'
import tokenHelper from '@/app/helpers/token.helper'
import { useDebounceFn } from '@vueuse/core'

export type { AxiosInstance }

interface ApiResponse<T = any> {
	data: T
	status: number
	statusText: string
	headers: any
	config: InternalAxiosRequestConfig
}

interface ApiError {
	message: string
	status?: number
	code?: string
	data?: any
}

export class AxiosService {
	public readonly instance: AxiosInstance
	public cluster: string

	constructor(cluster = '') {
		this.cluster = cluster
		this.instance = axios.create({
			baseURL: '',
			timeout: 30000,
			headers: {
				'Content-Type': 'application/json',
				Accept: 'application/json'
			},
			withCredentials: true,
			paramsSerializer: {
				indexes: null
			}
		})

		this.setupInterceptors()
	}

	private setupInterceptors(): void {
		// Request interceptor
		this.instance.interceptors.request.use(
			(config: InternalAxiosRequestConfig) => {
				if (this.cluster && !config.headers['x-cluster-url']) {
					config.headers['x-cluster-url'] = this.cluster
				}

				const url = config.url || ''
				const isLoginPath = url.startsWith('/auth/login') || url.startsWith('auth/login')
				if (
					!isLoginPath &&
					!config.headers.Authorization &&
					!config.headers.authorization
				) {
					const token = tokenHelper.getAccessToken()
					if (token) {
						config.headers.Authorization = `Bearer ${token}`
					}
				}

				return config
			},
			(error) => Promise.reject(error)
		)

		// Response interceptor
		this.instance.interceptors.response.use(
			(response: AxiosResponse) => this.handleResponse(response),
			(error) => this.handleError(error)
		)
	}

	private handleResponse(response: AxiosResponse): ApiResponse {
		showSuccessMessage(response)
		return {
			data: response.data,
			status: response.status,
			statusText: response.statusText,
			headers: response.headers,
			config: response.config
		}
	}

	private async handleError(error: unknown): Promise<any> {
		if (axios.isAxiosError(error)) {
			const originalRequest = error.config as
				| (InternalAxiosRequestConfig & { _retry?: boolean })
				| undefined

			if (error.response?.status === 401 && originalRequest) {
				const isAuthPath =
					originalRequest.url?.startsWith('/auth') ||
					originalRequest.url?.startsWith('auth')

				if (!isAuthPath && !originalRequest._retry) {
					originalRequest._retry = true
					try {
						const authStore = useAuthStore()
						const newToken = await authStore.refreshAccessToken()
						if (newToken) {
							originalRequest.headers.Authorization = `Bearer ${newToken}`
							return this.instance(originalRequest)
						}
					} catch {
						// refresh failed
					}
				}

				// If not refreshable or refresh failed
				try {
					const authStore = useAuthStore()
					await authStore.logout()
				} catch {
					// ignore
				}
				return Promise.reject(error.response)
			}

			if (error.response?.status === 422) {
				const err = Promise.reject(error.response)
				return err
			}

			const apiError: ApiError = {
				message: error.response?.data?.message ?? error.message,
				status: error.response?.status ?? (error.code as any),
				code: error.code,
				data: error.response?.data
			}

			return Promise.reject(apiError)
		}

		return Promise.reject({
			message: 'An unknown error occurred',
			code: 'UNKNOWN_ERROR'
		})
	}

	public async get<T>(
		url: string,
		config?: AxiosRequestConfig
	): Promise<AxiosResponse<T> | undefined> {
		try {
			const response = await this.instance.get<T>(url, config)
			return response
		} catch (error) {
			handleErrorMessage(error as any)
		}
	}

	public async post<T>(
		url: string,
		data?: any,
		config?: AxiosRequestConfig
	): Promise<AxiosResponse<T> | undefined> {
		try {
			const response = await this.instance.post<T>(url, data, config)
			return response
		} catch (error) {
			const isLogout = url.includes('/auth/logout') || url.includes('auth/logout')
			if (!isLogout) {
				handleErrorMessage(error as any)
			}
			throw error
		}
	}

	public async put<T>(
		url: string,
		data?: any,
		config?: AxiosRequestConfig
	): Promise<AxiosResponse<T> | undefined> {
		try {
			const response = await this.instance.put<T>(url, data, config)
			return response
		} catch (error: any) {
			handleErrorMessage(error)
			throw error
		}
	}

	public async delete<T>(
		url: string,
		config?: AxiosRequestConfig
	): Promise<AxiosResponse<T> | undefined> {
		try {
			const response = await this.instance.delete<T>(url, config)
			return response
		} catch (error: any) {
			handleErrorMessage(error)
		}
	}

	public async patch<T>(
		url: string,
		data?: any,
		config?: AxiosRequestConfig
	): Promise<AxiosResponse<T>> {
		const response = await this.instance.patch<T>(url, data, config)
		return response
	}
}

const httpInstances = new Map<string, AxiosService>()

// Export a singleton instance targeting Nitro server directly
export const http = new AxiosService()
httpInstances.set('', http)

export function createHTTPInstance(cluster?: string): AxiosService {
	const normalizedCluster = cluster ?? ''

	if (httpInstances.has(normalizedCluster)) {
		return httpInstances.get(normalizedCluster) as AxiosService
	}

	const instance = new AxiosService(normalizedCluster)
	httpInstances.set(normalizedCluster, instance)

	return instance
}

export const handleErrorMessage = useDebounceFn((error: ApiError) => {
	const payload = { title: '', message: '' }

	if (error.data) {
		if (typeof error.data == 'object') {
			payload.title = error.data.error?.title || 'Error'
			payload.message = error.data.error?.detail || ''
		}

		if (typeof error.data == 'string') {
			payload.title = 'Error Validation'
			payload.message = error.data
		}
	} else {
		payload.title = error.code as any
		payload.message = error.message
	}

	const appStore = useAppStore()
	appStore.showError(payload.title, payload.message)
}, 500)

export const showSuccessMessage = useDebounceFn((response?: AxiosResponse) => {
	const messagefor = ['post', 'put', 'delete']
	const apiExcludePath = ['/auth/login', '/auth/logout']
	const method = response?.config?.method ?? 'get'
	const url = response?.config?.url ?? '/'

	if (messagefor.includes(method) && !apiExcludePath.includes(url)) {
		let methodstr = 'Add'
		if (method == 'post') methodstr = 'Add'
		if (method == 'put') methodstr = 'Update'
		if (method == 'delete') methodstr = 'Delete'

		const payload = { title: `Success ${methodstr} Data`, message: '' }
		const appStore = useAppStore()
		appStore.showSuccess(payload.title, payload.message)
	}
}, 500)

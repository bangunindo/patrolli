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
import { useDebounceFn } from '@vueuse/core'

export type { AxiosInstance }

declare module 'axios' {
	export interface AxiosRequestConfig {
		suppressErrorStatuses?: number[]
		rethrowErrors?: boolean
	}
}

function shouldShowErrorToast(error: unknown, config?: AxiosRequestConfig): boolean {
	const status = (error as ApiError | undefined)?.status
	return typeof status !== 'number' || !config?.suppressErrorStatuses?.includes(status)
}

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

const AUTH_PATH_RE = /^\/?(?:api\/)?auth(?:\/|$)/
const AUTH_LIFECYCLE_RE = /\/?(?:api\/)?auth\/(?:login|logout|me|refresh|keycloak|oidc)(?:\/|$)/
const SILENT_AUTH_RE = /\/?(?:api\/)?auth\/(?:logout|refresh)/

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
		this.instance.interceptors.request.use(
			async (config: InternalAxiosRequestConfig) => {
				if (this.cluster && !config.headers['x-cluster-url']) {
					config.headers['x-cluster-url'] = this.cluster
				}

				const isAuthLifecyclePath = AUTH_LIFECYCLE_RE.test(config.url ?? '')
				if (!isAuthLifecyclePath) {
					try {
						const authStore = useAuthStore()
						if (authStore.hasActiveSession() && authStore.isAccessTokenExpiringSoon()) {
							await authStore.ensureValidAccessToken()
						}
					} catch {
						// Proceed if Pinia store is not active in isolated contexts
					}
				}

				return config
			},
			(error) => Promise.reject(error)
		)

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
				const isAuthLifecyclePath = AUTH_LIFECYCLE_RE.test(originalRequest.url ?? '')

				if (!isAuthLifecyclePath && !originalRequest._retry) {
					originalRequest._retry = true
					try {
						const authStore = useAuthStore()
						const refreshed = await authStore.refreshAccessToken()
						if (refreshed) {
							return this.instance(originalRequest)
						}
					} catch {
						// refresh failed
					}
				}

				return Promise.reject(error.response)
			}

			if (error.response?.status === 422) {
				return Promise.reject(error.response)
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
			if (shouldShowErrorToast(error, config)) {
				handleErrorMessage(error as any)
			}
			if (config?.rethrowErrors) {
				throw error
			}
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
			if (!SILENT_AUTH_RE.test(url) && shouldShowErrorToast(error, config)) {
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
			if (shouldShowErrorToast(error, config)) {
				handleErrorMessage(error)
			}
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
			if (shouldShowErrorToast(error, config)) {
				handleErrorMessage(error)
			}
			if (config?.rethrowErrors) {
				throw error
			}
		}
	}

	public async patch<T>(
		url: string,
		data?: any,
		config?: AxiosRequestConfig
	): Promise<AxiosResponse<T>> {
		try {
			const response = await this.instance.patch<T>(url, data, config)
			return response
		} catch (error: any) {
			if (shouldShowErrorToast(error, config)) {
				handleErrorMessage(error)
			}
			throw error
		}
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
	const method = response?.config?.method ?? 'get'
	const url = response?.config?.url ?? '/'

	if (messagefor.includes(method) && !AUTH_PATH_RE.test(url)) {
		const methodstr = method === 'put' ? 'Update' : method === 'delete' ? 'Delete' : 'Add'
		const appStore = useAppStore()
		appStore.showSuccess(`Success ${methodstr} Data`, '')
	}
}, 500)

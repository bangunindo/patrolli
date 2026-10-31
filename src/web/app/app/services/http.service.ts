// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { useAppStore } from '@/stores/app.store'
import type { AxiosRequestConfig } from 'axios'
import { getActivePinia } from 'pinia'
import { createHTTPInstance } from '@/app/services/axios.service'

export class HttpService {
	public async get<T = any>(
		path: string,
		config?: AxiosRequestConfig,
		clusterURLs?: string[]
	): Promise<T | undefined> {
		const cluster = this.getCluster(clusterURLs)
		const instance = createHTTPInstance(cluster)
		const response = await instance.get<T>(path, config)
		return response?.data
	}

	public async post<T = any>(
		path: string,
		data?: any,
		config?: AxiosRequestConfig,
		clusterURLs?: string[]
	): Promise<T | undefined> {
		const cluster = this.getCluster(clusterURLs)
		const instance = createHTTPInstance(cluster)
		const response = await instance.post<T>(path, data, config)
		return response?.data
	}

	public async put<T = any>(
		path: string,
		data?: any,
		config?: AxiosRequestConfig,
		clusterURLs?: string[]
	): Promise<T | undefined> {
		const cluster = this.getCluster(clusterURLs)
		const instance = createHTTPInstance(cluster)
		const response = await instance.put<T>(path, data, config)
		return response?.data
	}

	public async delete<T = any>(
		path: string,
		config?: AxiosRequestConfig,
		clusterURLs?: string[]
	): Promise<T | undefined> {
		const cluster = this.getCluster(clusterURLs)
		const instance = createHTTPInstance(cluster)
		const response = await instance.delete<T>(path, config)
		return response?.data
	}

	public async patch<T = any>(
		path: string,
		data?: any,
		config?: AxiosRequestConfig,
		clusterURLs?: string[]
	): Promise<T | undefined> {
		const cluster = this.getCluster(clusterURLs)
		const instance = createHTTPInstance(cluster)
		const response = await instance.patch<T>(path, data, config)
		return response?.data
	}

	private getCluster(clusterURLs?: string[]): string | undefined {
		if (clusterURLs && clusterURLs.length > 0) {
			return clusterURLs.join(',')
		}

		if (getActivePinia()) {
			try {
				const store = useAppStore()
				if (store?.activeClusterURL) {
					return store.activeClusterURL
				}
			} catch {
				// Pinia store not initialized yet
			}
		}

		return undefined
	}
}

export const httpService = new HttpService()

export const httpHelper = httpService

export default httpService

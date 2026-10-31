// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import axios, { type AxiosInstance, type AxiosRequestConfig, type AxiosResponse } from 'axios'
import fs from 'node:fs'
import http from 'node:http'
import https from 'node:https'
import path from 'node:path'
import { getKeycloakConfig } from './cluster.service'
import { getSslConfig } from './ssl.service'

let cachedClient: AxiosInstance | null = null

/**
 * Creates an HTTPS Agent configured with SSL certificates and verification rules from config-fe.yaml
 */
export function createHttpsAgent(): https.Agent {
	const sslConfig = getSslConfig()
	const kcConfig = getKeycloakConfig()

	const rejectUnauthorized =
		sslConfig?.reject_unauthorized !== undefined
			? sslConfig.reject_unauthorized
			: process.env.NODE_ENV === 'production'

	const caList: string[] = []
	const caCandidatePaths = [sslConfig?.ca, kcConfig?.ca_cert_path].filter(Boolean) as string[]

	for (const candidate of caCandidatePaths) {
		const resolvedPath = path.isAbsolute(candidate)
			? candidate
			: path.resolve(process.cwd(), candidate)
		try {
			if (fs.existsSync(resolvedPath)) {
				caList.push(fs.readFileSync(resolvedPath, 'utf-8'))
			}
		} catch {
			// Skip candidate if unreadable
		}
	}

	return new https.Agent({
		rejectUnauthorized,
		keepAlive: true,
		ca: caList.length > 0 ? caList : undefined
	})
}

/**
 * Creates an HTTP Agent with keepAlive enabled
 */
export function createHttpAgent(): http.Agent {
	return new http.Agent({
		keepAlive: true
	})
}

/**
 * Returns or creates the singleton server-side Axios client configured with custom TLS agents
 */
export function getServerHttpClient(): AxiosInstance {
	if (cachedClient) {
		return cachedClient
	}

	cachedClient = axios.create({
		httpAgent: createHttpAgent(),
		httpsAgent: createHttpsAgent(),
		validateStatus: () => true, // Return all HTTP statuses without throwing
		timeout: 30000,
		maxBodyLength: 50 * 1024 * 1024, // 🔒 50 MB safe body limit
		maxContentLength: 50 * 1024 * 1024 // 🔒 50 MB safe content limit
	})

	return cachedClient
}

/**
 * Functional wrapper to execute an HTTP request using the server HTTP client
 */
export async function executeHttpRequest<T = any>(
	config: AxiosRequestConfig
): Promise<AxiosResponse<T>> {
	const client = getServerHttpClient()
	return client.request<T>(config)
}

export const serverHttpClient: AxiosInstance = new Proxy({} as AxiosInstance, {
	get(_target, prop, receiver) {
		const client = getServerHttpClient()
		const value = Reflect.get(client, prop, receiver)
		return typeof value === 'function' ? value.bind(client) : value
	}
})

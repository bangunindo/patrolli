// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import axios, { type AxiosInstance, type AxiosRequestConfig, type AxiosResponse } from 'axios'
import http from 'node:http'
import https from 'node:https'
import { getOidcConfig } from './cluster.service'
import { getDefaultCacheCheckTtlMs } from './config-file.service'
import { getTlsConnectCacheKey, resolveTlsConnectOptions } from './ssl.service'

let cachedClient: {
	key: string
	checkedAt: number
	client: AxiosInstance
	httpAgent: http.Agent
	httpsAgent: https.Agent
} | null = null

export function resolveOidcCaPath(): string | undefined {
	try {
		return getOidcConfig()?.ca_cert_path
	} catch {
		return undefined
	}
}

export function drainAgentIdleSockets(agent?: http.Agent): void {
	if (!agent) return
	for (const sockets of Object.values(agent.freeSockets)) {
		for (const socket of sockets || []) {
			socket.destroy()
		}
	}
}

export function clearServerHttpClientCache(): void {
	if (!cachedClient) return
	cachedClient.httpAgent.destroy()
	cachedClient.httpsAgent.destroy()
	cachedClient = null
}

export function createHttpsAgent(): https.Agent {
	const oidcCaPath = resolveOidcCaPath()
	const { rejectUnauthorized, ca } = resolveTlsConnectOptions([oidcCaPath])

	return new https.Agent({
		rejectUnauthorized,
		keepAlive: true,
		ca
	})
}

export function createHttpAgent(): http.Agent {
	return new http.Agent({
		keepAlive: true
	})
}

export function getServerHttpClient(
	checkTtlMs: number = getDefaultCacheCheckTtlMs()
): AxiosInstance {
	const now = Date.now()
	if (checkTtlMs > 0 && cachedClient && now - cachedClient.checkedAt < checkTtlMs) {
		return cachedClient.client
	}

	const key = getTlsConnectCacheKey([resolveOidcCaPath()])
	if (cachedClient && cachedClient.key === key) {
		cachedClient.checkedAt = now
		return cachedClient.client
	}

	const prevHttpsAgent = cachedClient?.httpsAgent
	const httpAgent = cachedClient?.httpAgent ?? createHttpAgent()
	const httpsAgent = createHttpsAgent()
	const client = axios.create({
		httpAgent,
		httpsAgent,
		validateStatus: () => true,
		timeout: 30000,
		maxBodyLength: 50 * 1024 * 1024,
		maxContentLength: 50 * 1024 * 1024
	})

	cachedClient = { key, checkedAt: now, client, httpAgent, httpsAgent }
	drainAgentIdleSockets(prevHttpsAgent)

	return client
}

export async function executeHttpRequest<T = any>(
	config: AxiosRequestConfig
): Promise<AxiosResponse<T>> {
	const client = getServerHttpClient()
	return client.request<T>(config)
}

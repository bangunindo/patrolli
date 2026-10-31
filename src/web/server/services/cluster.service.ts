// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { logClusterError, logClusterForward, logClusterResponse } from './logger.service'
import { handleSingleClusterResponse, mergeDynamicResponses } from '@@/server/utils/response'
import { executeHttpRequest } from '@@/server/services/apibase.service'
import type { ICluster, IKeycloakConfig } from '@@/shared/types/cluster.type'
import { type H3Event, getQuery, getRequestHeaders, readBody, setResponseStatus } from 'h3'
import fs from 'node:fs'
import path from 'node:path'
import process from 'node:process'
import YAML from 'yaml'

declare const useRuntimeConfig: (event?: H3Event) => any

/**
 * Parses raw cluster configuration from various formats (YAML string, JSON string, Array of objects, Record map).
 * Supports:
 * - config-fe.yaml file content:
 *     clusters:
 *       - name: 'patrolli'
 *         url: 'http://patrolli:8080'
 * - Environment variable CLUSTERS / CLUSTER:
 *     - name: 'patrolli'
 *       url: 'http://patrolli:8080'
 *     - name: 'patrolli1'
 *       url: 'http://patrolli1:8080'
 * - JSON string: [ { name: 'patrolli1', url: 'http://patrolli1:8080' } ]
 * - Object / Map: { patrolli1: 'http://patrolli1:8080' }
 */
export function parseClustersConfig(raw: any): ICluster[] {
	if (!raw) return []

	if (typeof raw === 'string') {
		const trimmed = raw.trim()
		if (!trimmed) return []

		try {
			// Handle escaped newlines if passed from single-line env string
			const unescaped =
				trimmed.includes('\\n') && !trimmed.includes('\n')
					? trimmed.replace(/\\n/g, '\n')
					: trimmed
			const parsed = YAML.parse(unescaped)
			return parseClustersConfig(parsed)
		} catch {
			try {
				const jsonParsed = JSON.parse(trimmed)
				return parseClustersConfig(jsonParsed)
			} catch {
				return []
			}
		}
	}

	if (Array.isArray(raw)) {
		const entries: ICluster[] = []
		for (const item of raw) {
			if (!item || typeof item !== 'object') continue

			if (Array.isArray(item.clusters || item.cluster)) {
				entries.push(...parseClustersConfig(item.clusters || item.cluster))
			} else if (item.clusters && typeof item.clusters === 'object') {
				entries.push(...parseClustersConfig(item.clusters))
			} else if (item.name && item.url) {
				const cleanName = String(item.name).trim()
				const cleanUrl = String(item.url).trim().replace(/\/+$/, '')
				entries.push({
					key: item.key ? String(item.key).trim() : cleanName,
					name: cleanName,
					url: cleanUrl,
					clientPath: item.clientPath
						? String(item.clientPath).trim()
						: `/${cleanName.replace(/^\//, '')}`
				})
			}
		}
		return entries
	}

	if (typeof raw === 'object' && raw !== null) {
		if (raw.clusters || raw.cluster) {
			return parseClustersConfig(raw.clusters || raw.cluster)
		}

		const result: ICluster[] = []
		for (const [key, value] of Object.entries(raw)) {
			const cleanKey = key.trim()
			if (typeof value === 'string' && value.trim()) {
				result.push({
					key: cleanKey,
					name: cleanKey,
					url: value.trim().replace(/\/+$/, ''),
					clientPath: `/${cleanKey.replace(/^\//, '')}`
				})
			} else if (typeof value === 'object' && value !== null && (value as any).url) {
				const name = (value as any).name ? String((value as any).name).trim() : cleanKey
				const url = String((value as any).url)
					.trim()
					.replace(/\/+$/, '')
				result.push({
					key: (value as any).key ? String((value as any).key).trim() : cleanKey,
					name,
					url,
					clientPath: (value as any).clientPath
						? String((value as any).clientPath).trim()
						: `/${name.replace(/^\//, '')}`
				})
			}
		}
		return result
	}

	return []
}

export interface IAppConfig {
	clusters?: any
	auth?: {
		keycloak?: IKeycloakConfig
	}
}

export function getAppConfigFileContent(configPath?: string): { parsed: any; raw: string } | null {
	const explicitPath =
		configPath ||
		process.env.CONFIG_FILE ||
		process.env.CONFIG_PATH ||
		process.env.FRONTEND_CONFIG

	const candidatePaths = explicitPath
		? [explicitPath]
		: [
				path.resolve(process.cwd(), 'config-fe.yaml'),
				path.resolve(process.cwd(), 'config-fe.yml'),
				'/app/config-fe.yaml',
				'/config-fe.yaml'
			]

	for (const filePath of candidatePaths) {
		try {
			if (fs.existsSync(filePath)) {
				const content = fs.readFileSync(filePath, 'utf-8')
				if (content.trim()) {
					const parsed = YAML.parse(content)
					return { parsed, raw: content }
				}
			}
		} catch {
			// Skip to next candidate if error reading file
		}
	}
	return null
}

export function getKeycloakConfig(configPath?: string): IKeycloakConfig | null {
	const fileConfig = getAppConfigFileContent(configPath)
	if (fileConfig?.parsed?.auth?.keycloak) {
		const kc = fileConfig.parsed.auth.keycloak
		return {
			host: String(kc.host || '')
				.trim()
				.replace(/\/+$/, ''),
			client_id: String(kc.client_id || '').trim(),
			client_secret: String(kc.client_secret || '').trim(),
			redirect_uri: String(kc.redirect_uri || '').trim(),
			ca_cert_path: kc.ca_cert_path ? String(kc.ca_cert_path).trim() : undefined
		}
	}
	return null
}

export function getClustersConfig(event?: H3Event, configPath?: string): ICluster[] {
	const fileConfig = getAppConfigFileContent(configPath)
	if (fileConfig?.parsed) {
		const clusters = parseClustersConfig(fileConfig.parsed)
		if (clusters.length > 0) {
			return clusters
		}
	}

	// Fallback to environment variables
	let rawClusters: any =
		process.env.CLUSTERS ||
		process.env.CLUSTER ||
		process.env.NUXT_CLUSTERS ||
		process.env.NUXT_CLUSTER

	if (!rawClusters) {
		try {
			const config = event ? useRuntimeConfig(event) : useRuntimeConfig()
			rawClusters = config?.clusters || config?.cluster
		} catch {
			// fallback if runtime config is not available
		}
	}

	return parseClustersConfig(rawClusters)
}

export function resolveClusterTargets(
	clusterHeader: string | null | undefined,
	allClusters: ICluster[]
): ICluster[] {
	if (!clusterHeader || clusterHeader.trim() === '' || clusterHeader === 'all') {
		return allClusters
	}

	const requestedKeys = clusterHeader
		.split(',')
		.map((k) => k.trim())
		.filter(Boolean)
	const matchedClusters: ICluster[] = []

	for (const req of requestedKeys) {
		const cleanKey = req.replace(/^\//, '')
		const normalizedReqUrl = req.replace(/\/+$/, '')
		const matched = allClusters.find(
			(c) =>
				c.key === cleanKey ||
				c.name === cleanKey ||
				c.url === req ||
				c.url.replace(/\/+$/, '') === normalizedReqUrl ||
				c.clientPath === req ||
				c.clientPath === `/${cleanKey}`
		)
		if (matched) {
			matchedClusters.push(matched)
		}
	}

	return matchedClusters.length > 0 ? matchedClusters : allClusters
}

export async function forwardToCluster(
	cluster: ICluster,
	targetPath: string,
	options: {
		method: string
		headers?: Record<string, any>
		query?: Record<string, any>
		body?: any
	}
): Promise<{ status: number; headers: Headers; data: any }> {
	const targetUrl = new URL(
		targetPath.startsWith('/') ? targetPath : `/${targetPath}`,
		cluster.url
	)
	if (options.query) {
		Object.entries(options.query).forEach(([k, v]) => {
			if (v !== undefined && v !== null && k !== 'cluster') {
				if (Array.isArray(v)) {
					v.forEach((val) => targetUrl.searchParams.append(k, String(val)))
				} else {
					targetUrl.searchParams.set(k, String(v))
				}
			}
		})
	}

	const reqHeaders: Record<string, string> = {}
	if (options.headers?.cookie) reqHeaders.cookie = options.headers.cookie
	let authHeader = options.headers?.authorization || options.headers?.Authorization
	if (!authHeader && options.headers?.cookie) {
		const match =
			typeof options.headers.cookie === 'string'
				? options.headers.cookie.match(/(?:^|;\s*)access_token=([^;]+)/)
				: null
		if (match && match[1]) {
			authHeader = `Bearer ${decodeURIComponent(match[1])}`
		}
	}
	if (authHeader) reqHeaders.authorization = authHeader
	if (options.headers?.['content-type'])
		reqHeaders['content-type'] = options.headers['content-type']
	if (options.headers?.accept) reqHeaders.accept = options.headers.accept

	if (options.body && options.method !== 'GET' && options.method !== 'HEAD') {
		if (!reqHeaders['content-type']) {
			reqHeaders['content-type'] = 'application/json'
		}
	}

	const startTime = Date.now()
	logClusterForward(options.method, cluster.name, targetUrl.toString())

	try {
		const axiosRes = await executeHttpRequest({
			method: options.method,
			url: targetUrl.toString(),
			headers: reqHeaders,
			data:
				options.body && options.method !== 'GET' && options.method !== 'HEAD'
					? options.body
					: undefined,
			validateStatus: () => true
		})
		const durationMs = Date.now() - startTime

		// Map Axios response headers into standard Headers instance
		const resHeaders = new Headers()
		if (axiosRes.headers) {
			for (const [key, val] of Object.entries(axiosRes.headers)) {
				if (val !== undefined && val !== null) {
					if (Array.isArray(val)) {
						val.forEach((v) => resHeaders.append(key, String(v)))
					} else {
						resHeaders.set(key, String(val))
					}
				}
			}
			if (Array.isArray(axiosRes.headers['set-cookie'])) {
				;(resHeaders as any).getSetCookie = () => axiosRes.headers['set-cookie']
			}
		}

		const data = axiosRes.data

		logClusterResponse(
			cluster.name,
			options.method,
			targetUrl.toString(),
			axiosRes.status,
			durationMs,
			data
		)

		return {
			status: axiosRes.status,
			headers: resHeaders,
			data
		}
	} catch (error: any) {
		const durationMs = Date.now() - startTime
		logClusterError(cluster.name, options.method, targetUrl.toString(), error, durationMs)

		return {
			status: 502,
			headers: new Headers(),
			data: {
				status_code: 502,
				success: false,
				error: {
					title: 'Bad Gateway',
					detail: `Failed to communicate with cluster ${cluster.name}`
				}
			}
		}
	}
}

export async function handleClusterMutation(event: H3Event) {
	const method = event.method
	const pathParam = event.context.params?.path || ''
	const fullApiPath = `/api/${pathParam}`
	const allClusters = getClustersConfig(event)
	const reqHeaders = getRequestHeaders(event)
	const query = getQuery(event)
	const clusterHeader =
		(reqHeaders['x-cluster-url'] as string | undefined) ||
		(query.cluster as string | undefined) ||
		null

	const targetClusters = resolveClusterTargets(clusterHeader, allClusters)

	if (targetClusters.length === 0) {
		const errorPayload = {
			status_code: 502,
			success: false,
			error: {
				title: 'Bad Gateway',
				detail: 'No clusters configured. Please configure clusters in config-fe.yaml or environment variables.'
			}
		}
		setResponseStatus(event, 502)
		return errorPayload
	}

	let body: any = undefined
	if (method !== 'GET' && method !== 'HEAD') {
		try {
			body = await readBody(event)
		} catch {
			// Body could be empty or raw
		}
	}

	// 1. Single cluster target
	const singleTarget = targetClusters[0]
	if (targetClusters.length === 1 && singleTarget) {
		const res = await forwardToCluster(singleTarget, fullApiPath, {
			method,
			headers: reqHeaders,
			query,
			body
		})
		return handleSingleClusterResponse(event, singleTarget, res)
	}

	// 2. Multi-cluster mutations (POST, PUT, DELETE, PATCH)
	const mutationResponses = await Promise.all(
		targetClusters.map(async (cluster) => {
			const res = await forwardToCluster(cluster, fullApiPath, {
				method,
				headers: reqHeaders,
				query,
				body
			})
			return { cluster, data: res.data, status: res.status }
		})
	)

	const merged = mergeDynamicResponses(mutationResponses)
	const finalStatus = merged.status_code || 200
	setResponseStatus(event, finalStatus)
	return merged
}

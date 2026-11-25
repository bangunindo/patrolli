// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { logClusterError, logClusterForward, logClusterResponse } from './logger.service'
import {
	executeClusterRequestsWithAuthRetry,
	executeSingleClusterRequestWithAuthRetry
} from './auth-refresh.service'
import { handleSingleClusterResponse, mergeDynamicResponses } from '@@/server/utils/response'
import { executeHttpRequest } from '@@/server/services/apibase.service'
import { extractProxyContext } from '@@/server/modules/proxy/proxy.utils'
import type {
	ICluster,
	IKeycloakConfig,
	IOidcConfig,
	OidcTokenEndpointAuthMethod,
	OidcTokenSource
} from '@@/shared/types/cluster.type'
import { type H3Event, readBody, setResponseStatus } from 'h3'
import YAML from 'yaml'
import { getCandidateConfigFilePaths, readCachedYamlConfigFile } from './config-file.service'

/**
 * Parses raw cluster configuration from various formats (YAML string, JSON string, Array of objects, Record map).
 * Supports:
 * - config-web.yaml file content:
 *     clusters:
 *       - name: 'patrolli'
 *         url: 'http://patrolli:8080'
 * - JSON string: [ { name: 'patrolli1', url: 'http://patrolli1:8080' } ]
 * - Object / Map: { patrolli1: 'http://patrolli1:8080' }
 */
function createClusterEntry(item: Record<string, any>, fallbackKey?: string): ICluster {
	const cleanName = item.name ? String(item.name).trim() : String(fallbackKey || '').trim()
	const cleanKey = item.key ? String(item.key).trim() : String(fallbackKey || cleanName).trim()
	const cleanUrl = String(item.url).trim().replace(/\/+$/, '')

	return {
		key: cleanKey,
		name: cleanName,
		url: cleanUrl,
		clientPath: item.clientPath
			? String(item.clientPath).trim()
			: `/${cleanName.replace(/^\//, '')}`
	}
}

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
				entries.push(createClusterEntry(item))
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
				result.push(createClusterEntry({ url: value }, cleanKey))
			} else if (typeof value === 'object' && value !== null && (value as any).url) {
				result.push(createClusterEntry(value as Record<string, any>, cleanKey))
			}
		}
		return result
	}

	return []
}

const loggedConfigWarnings = new Set<string>()
const clustersByParsedConfig = new WeakMap<object, ICluster[]>()
const oidcByParsedConfig = new WeakMap<object, IOidcConfig | null>()

export function resetConfigWarnings(): void {
	loggedConfigWarnings.clear()
}

function warnConfigOnce(message: string): void {
	if (loggedConfigWarnings.has(message)) return
	loggedConfigWarnings.add(message)
	console.warn(message)
}

export function getAppConfigFileContent(configPath?: string): { parsed: any; raw: string } {
	const loaded = readCachedYamlConfigFile(configPath)
	if (loaded) {
		return { parsed: loaded.parsed, raw: loaded.raw }
	}

	const candidatePaths = getCandidateConfigFilePaths(configPath)
	throw new Error(
		`[Patrolli-FE Config] Configuration file not found or empty. Checked paths: ${candidatePaths.join(', ')}`
	)
}

function normalizeTokenSource(value: unknown, fallback: OidcTokenSource): OidcTokenSource {
	if (value === 'access_token' || value === 'id_token') {
		return value
	}
	if (value !== undefined && value !== null && String(value).trim() !== '') {
		warnConfigOnce(
			`[Patrolli-FE Config] Unknown auth.oidc.token_source "${String(value)}"; falling back to "${fallback}".`
		)
	}
	return fallback
}

function normalizeTokenEndpointAuthMethod(value: unknown): OidcTokenEndpointAuthMethod | undefined {
	if (value === 'client_secret_post' || value === 'client_secret_basic') {
		return value
	}
	if (value !== undefined && value !== null && String(value).trim() !== '') {
		warnConfigOnce(
			`[Patrolli-FE Config] Unknown auth.oidc.token_endpoint_auth_method "${String(value)}"; falling back to auto-detection.`
		)
	}
	return undefined
}

export function getOidcConfig(configPath?: string): IOidcConfig | null {
	const fileConfig = getAppConfigFileContent(configPath)
	const parsed = fileConfig?.parsed
	const canMemoize = parsed !== null && typeof parsed === 'object'
	if (canMemoize && oidcByParsedConfig.has(parsed)) {
		return oidcByParsedConfig.get(parsed) ?? null
	}

	const auth = parsed?.auth
	let resolved: IOidcConfig | null = null

	if (auth?.oidc) {
		const oidc = auth.oidc
		const clientSecret = String(oidc.client_secret || '').trim()
		const redirectUri = String(oidc.redirect_uri || '').trim()
		resolved = {
			issuer: String(oidc.issuer || '').trim(),
			client_id: String(oidc.client_id || '').trim(),
			client_secret: clientSecret || undefined,
			redirect_uri: redirectUri || undefined,
			scopes: String(oidc.scopes ?? '').trim() || 'openid profile email',
			token_source: normalizeTokenSource(oidc.token_source, 'id_token'),
			token_endpoint_auth_method: normalizeTokenEndpointAuthMethod(
				oidc.token_endpoint_auth_method
			),
			display_name: String(oidc.display_name ?? '').trim() || 'SSO',
			ca_cert_path: oidc.ca_cert_path ? String(oidc.ca_cert_path).trim() : undefined
		}
	} else if (auth?.keycloak) {
		warnConfigOnce(
			'[Patrolli-FE Config] "auth.keycloak" is deprecated in config-web.yaml; migrate to "auth.oidc".'
		)
		const kc = auth.keycloak
		const clientSecret = String(kc.client_secret || '').trim()
		const redirectUri = String(kc.redirect_uri || '').trim()
		resolved = {
			issuer: String(kc.issuer || kc.host || '')
				.trim()
				.replace(/\/+$/, ''),
			client_id: String(kc.client_id || '').trim(),
			client_secret: clientSecret || undefined,
			redirect_uri: redirectUri || undefined,
			scopes: String(kc.scopes ?? '').trim() || 'openid profile email',
			token_source: normalizeTokenSource(kc.token_source, 'access_token'),
			token_endpoint_auth_method: normalizeTokenEndpointAuthMethod(
				kc.token_endpoint_auth_method
			),
			display_name: String(kc.display_name ?? '').trim() || 'Keycloak',
			ca_cert_path: kc.ca_cert_path ? String(kc.ca_cert_path).trim() : undefined
		}
	}

	if (canMemoize) {
		oidcByParsedConfig.set(parsed, resolved)
	}
	return resolved
}

export function getKeycloakConfig(configPath?: string): IKeycloakConfig | null {
	const oidc = getOidcConfig(configPath)
	if (!oidc) return null
	return {
		host: oidc.issuer.replace(/\/+$/, ''),
		client_id: oidc.client_id,
		client_secret: oidc.client_secret || '',
		redirect_uri: oidc.redirect_uri || '',
		ca_cert_path: oidc.ca_cert_path
	}
}

export function getClustersConfig(_event?: H3Event, configPath?: string): ICluster[] {
	const fileConfig = getAppConfigFileContent(configPath)
	const parsed = fileConfig?.parsed
	if (parsed !== null && typeof parsed === 'object') {
		const cached = clustersByParsedConfig.get(parsed)
		if (cached) {
			return cached
		}
		const resolved = parseClustersConfig(parsed)
		clustersByParsedConfig.set(parsed, resolved)
		return resolved
	}
	return parseClustersConfig(parsed)
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
	const rawPath = targetPath.startsWith('/') ? targetPath : `/${targetPath}`
	const baseUrl = new URL(cluster.url)
	const basePath = baseUrl.pathname.replace(/\/+$/, '')
	const finalPath = `${basePath}${rawPath}`
	const targetUrl = new URL(finalPath, cluster.url)
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
	const { method, fullApiPath, reqHeaders, query, clusterHeader } = extractProxyContext(event)
	const allClusters = getClustersConfig(event)
	const targetClusters = resolveClusterTargets(clusterHeader, allClusters)

	if (targetClusters.length === 0) {
		const errorPayload = {
			status_code: 502,
			success: false,
			error: {
				title: 'Bad Gateway',
				detail: 'No clusters configured. Please configure clusters in config-web.yaml or environment variables.'
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

	const singleTarget = targetClusters[0]
	if (targetClusters.length === 1 && singleTarget) {
		const { result } = await executeSingleClusterRequestWithAuthRetry(event, {
			item: singleTarget,
			reqHeaders,
			clusterHeader,
			execute: (cluster, headers) =>
				forwardToCluster(cluster, fullApiPath, {
					method,
					headers,
					query,
					body
				})
		})
		return handleSingleClusterResponse(event, singleTarget, result)
	}

	const { results: mutationResponses } = await executeClusterRequestsWithAuthRetry(event, {
		items: targetClusters,
		reqHeaders,
		clusterHeader,
		execute: async (cluster, headers) => {
			const res = await forwardToCluster(cluster, fullApiPath, {
				method,
				headers,
				query,
				body
			})
			return { cluster, data: res.data, status: res.status }
		}
	})

	const merged = mergeDynamicResponses(mutationResponses)
	const finalStatus = merged.status_code || 200
	setResponseStatus(event, finalStatus)
	return merged
}

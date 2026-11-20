// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import fs from 'node:fs'
import path from 'node:path'
import process from 'node:process'
import tls from 'node:tls'
import type { H3Event } from 'h3'
import { getRequestHeader } from 'h3'
import { getDefaultCacheCheckTtlMs, readCachedYamlConfigFile } from './config-file.service'

interface ISslConfig {
	enabled?: boolean
	cert?: string
	key?: string
	ca?: string
	reject_unauthorized?: boolean
}

interface ILoadedSslConfig {
	certPath: string
	keyPath: string
	cert: string
	key: string
	caPath?: string
	ca?: string
	rejectUnauthorized?: boolean
}

interface ICachedCaEntry {
	mtimeMs: number
	size: number
	checkedAt: number
	content: string
}

interface ICachedCaStatEntry {
	checkedAt: number
	fingerprint: string
}

const caCertCache = new Map<string, ICachedCaEntry>()
const caStatCache = new Map<string, ICachedCaStatEntry>()

export function clearCaCertCache(): void {
	caCertCache.clear()
	caStatCache.clear()
}

function readCaCertCached(
	resolvedPath: string,
	statTtlMs: number = getDefaultCacheCheckTtlMs()
): string | null {
	const now = Date.now()
	if (statTtlMs > 0) {
		const cached = caCertCache.get(resolvedPath)
		if (cached && now - cached.checkedAt < statTtlMs) {
			return cached.content
		}
	}

	try {
		if (!fs.existsSync(resolvedPath)) {
			caCertCache.delete(resolvedPath)
			return null
		}

		let stat: fs.Stats | null = null
		try {
			stat = fs.statSync(resolvedPath)
		} catch {
			stat = null
		}

		if (stat) {
			const cached = caCertCache.get(resolvedPath)
			if (cached && cached.mtimeMs === stat.mtimeMs && cached.size === stat.size) {
				cached.checkedAt = now
				return cached.content
			}
		}

		const content = fs.readFileSync(resolvedPath, 'utf-8')
		if (stat) {
			caCertCache.set(resolvedPath, {
				mtimeMs: stat.mtimeMs,
				size: stat.size,
				checkedAt: now,
				content
			})
		}
		return content
	} catch {
		return null
	}
}

export function getSslConfig(configPath?: string): ISslConfig | null {
	const loaded = readCachedYamlConfigFile(configPath)
	if (loaded?.parsed?.ssl && typeof loaded.parsed.ssl === 'object') {
		return loaded.parsed.ssl
	}
	return null
}

export function loadSslConfig(configPath?: string): ILoadedSslConfig | null {
	const ssl = getSslConfig(configPath)
	if (!ssl || ssl.enabled === false) {
		return null
	}

	if (!ssl.cert || !ssl.key) {
		return null
	}

	try {
		const certPath = path.isAbsolute(ssl.cert)
			? ssl.cert
			: path.resolve(process.cwd(), ssl.cert)

		const keyPath = path.isAbsolute(ssl.key) ? ssl.key : path.resolve(process.cwd(), ssl.key)

		const isSilentCommand =
			Boolean(process.env.VITEST) ||
			process.env.NODE_ENV === 'test' ||
			process.argv.some((arg) => arg.includes('prepare') || arg.includes('test'))

		if (!fs.existsSync(certPath) || !fs.existsSync(keyPath)) {
			if (!isSilentCommand) {
				console.warn(
					`[Patrolli-FE SSL] Certificate or key file not found: cert="${certPath}", key="${keyPath}"`
				)
			}
			return null
		}

		const cert = fs.readFileSync(certPath, 'utf-8')
		const key = fs.readFileSync(keyPath, 'utf-8')

		process.env.NITRO_SSL_CERT = cert
		process.env.NITRO_SSL_KEY = key

		let caPath: string | undefined
		let ca: string | undefined

		if (ssl.ca) {
			const resolvedCa = path.isAbsolute(ssl.ca)
				? ssl.ca
				: path.resolve(process.cwd(), ssl.ca)
			if (fs.existsSync(resolvedCa)) {
				caPath = resolvedCa
				ca = fs.readFileSync(resolvedCa, 'utf-8')
				process.env.NODE_EXTRA_CA_CERTS = resolvedCa
				if (!isSilentCommand) {
					console.log(`[Patrolli-FE SSL] Loaded CA certificate from "${caPath}"`)
				}
			} else if (!isSilentCommand) {
				console.warn(`[Patrolli-FE SSL] CA certificate file not found at "${resolvedCa}"`)
			}
		}

		if (ssl.reject_unauthorized !== undefined) {
			process.env.NODE_TLS_REJECT_UNAUTHORIZED = ssl.reject_unauthorized ? '1' : '0'
		} else if (process.env.NODE_ENV !== 'production') {
			// In local dev with local/self-signed certs, disable TLS rejection for internal fetch
			process.env.NODE_TLS_REJECT_UNAUTHORIZED = '0'
		}

		if (!isSilentCommand) {
			console.log(
				`[Patrolli-FE SSL] Loaded SSL certificate from "${certPath}" and key from "${keyPath}" (NODE_TLS_REJECT_UNAUTHORIZED=${process.env.NODE_TLS_REJECT_UNAUTHORIZED})`
			)
		}

		return {
			certPath,
			keyPath,
			cert,
			key,
			caPath,
			ca,
			rejectUnauthorized: ssl.reject_unauthorized
		}
	} catch (error) {
		console.warn('[Patrolli-FE SSL] Failed to load SSL certificate/key files:', error)
		return null
	}
}

export function parseForwardedProto(
	rawHeader: string | string[] | undefined
): 'http' | 'https' | null {
	const rawValue = Array.isArray(rawHeader) ? rawHeader[0] : rawHeader
	if (typeof rawValue !== 'string') return null
	const firstProto = rawValue.split(',')[0]?.trim().toLowerCase()
	if (firstProto === 'https' || firstProto === 'http') {
		return firstProto
	}
	return null
}

export function isRequestSecure(event?: H3Event): boolean {
	if (process.env.COOKIE_SECURE === 'true') return true
	if (process.env.COOKIE_SECURE === 'false') return false

	const sslConfig = getSslConfig()
	if (sslConfig && sslConfig.enabled !== false && sslConfig.cert && sslConfig.key) {
		return true
	}

	if (event) {
		const socket = event.node?.req?.socket as any
		if (socket?.encrypted) {
			return true
		}

		// Use the first token for multi-hop proxies (e.g. "https, http")
		if (parseForwardedProto(getRequestHeader(event, 'x-forwarded-proto')) === 'https') {
			return true
		}
	}

	return process.env.NODE_ENV === 'production'
}

function resolveTlsInputs(extraCaPaths: Array<string | undefined> = []): {
	rejectUnauthorized: boolean
	caPaths: string[]
} {
	const sslConfig = getSslConfig()
	const rejectUnauthorized =
		sslConfig?.reject_unauthorized !== undefined
			? sslConfig.reject_unauthorized
			: process.env.NODE_ENV === 'production'

	const seenPaths = new Set<string>()
	const caPaths: string[] = []
	const candidates = [sslConfig?.ca, ...extraCaPaths].filter(Boolean) as string[]

	for (const candidate of candidates) {
		const resolvedPath = path.isAbsolute(candidate)
			? candidate
			: path.resolve(process.cwd(), candidate)
		if (seenPaths.has(resolvedPath)) continue
		seenPaths.add(resolvedPath)
		caPaths.push(resolvedPath)
	}

	return { rejectUnauthorized, caPaths }
}

export function getTlsConnectCacheKey(
	extraCaPaths: Array<string | undefined> = [],
	statTtlMs: number = getDefaultCacheCheckTtlMs()
): string {
	const { rejectUnauthorized, caPaths } = resolveTlsInputs(extraCaPaths)
	const now = Date.now()
	const parts: string[] = [String(rejectUnauthorized)]

	for (const resolvedPath of caPaths) {
		if (statTtlMs > 0) {
			const cachedStat = caStatCache.get(resolvedPath)
			if (cachedStat && now - cachedStat.checkedAt < statTtlMs) {
				parts.push(cachedStat.fingerprint)
				continue
			}
		}

		let fingerprint: string
		try {
			if (fs.existsSync(resolvedPath)) {
				const stat = fs.statSync(resolvedPath)
				fingerprint = `${resolvedPath}:${stat.mtimeMs}:${stat.size}`
			} else {
				fingerprint = `${resolvedPath}:missing`
			}
		} catch {
			fingerprint = `${resolvedPath}:unknown`
		}

		caStatCache.set(resolvedPath, { checkedAt: now, fingerprint })
		parts.push(fingerprint)
	}

	return parts.join('|')
}

export function resolveTlsConnectOptions(extraCaPaths: Array<string | undefined> = []): {
	rejectUnauthorized: boolean
	ca?: string[]
} {
	const { rejectUnauthorized, caPaths } = resolveTlsInputs(extraCaPaths)
	const caList: string[] = []

	for (const resolvedPath of caPaths) {
		const content = readCaCertCached(resolvedPath)
		if (content !== null) {
			caList.push(content)
		}
	}

	return {
		rejectUnauthorized,
		// Extend Node's built-in root CAs rather than replacing them so internal CAs do not break public OIDC/HTTPS endpoints
		...(caList.length > 0 ? { ca: [...tls.rootCertificates, ...caList] } : {})
	}
}

// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import fs from 'node:fs'
import path from 'node:path'
import process from 'node:process'
import type { H3Event } from 'h3'
import { getRequestHeader } from 'h3'
import YAML from 'yaml'

export interface ISslConfig {
	enabled?: boolean
	cert?: string
	key?: string
	ca?: string
	reject_unauthorized?: boolean
}

export interface ILoadedSslConfig {
	certPath: string
	keyPath: string
	cert: string
	key: string
	caPath?: string
	ca?: string
	rejectUnauthorized?: boolean
}

function getCandidateConfigFilePaths(configPath?: string): string[] {
	const explicitPath =
		configPath ||
		process.env.CONFIG_FILE ||
		process.env.CONFIG_PATH ||
		process.env.FRONTEND_CONFIG

	return explicitPath
		? [explicitPath]
		: [
				path.resolve(process.cwd(), 'config-fe.yaml'),
				path.resolve(process.cwd(), 'config-fe.yml'),
				'/app/config-fe.yaml',
				'/config-fe.yaml'
			]
}

/**
 * Returns raw SSL configuration object from config-fe.yaml if present.
 */
export function getSslConfig(configPath?: string): ISslConfig | null {
	const candidatePaths = getCandidateConfigFilePaths(configPath)

	for (const filePath of candidatePaths) {
		try {
			if (fs.existsSync(filePath)) {
				const content = fs.readFileSync(filePath, 'utf-8')
				if (content.trim()) {
					const parsed = YAML.parse(content)
					if (parsed?.ssl && typeof parsed.ssl === 'object') {
						return parsed.ssl
					}
				}
			}
		} catch {
			// Skip to next candidate if error reading file
		}
	}
	return null
}

/**
 * Reads, resolves file paths, and verifies SSL certificate and key files.
 * Returns ready-to-use cert and key strings/paths for Nuxt devServer and Nitro SSL listeners.
 */
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

		// Populate Nitro node-server native environment variables
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

/**
 * Checks whether the incoming request or current environment is running securely under HTTPS.
 */
export function isRequestSecure(event?: H3Event): boolean {
	if (process.env.COOKIE_SECURE === 'true') return true
	if (process.env.COOKIE_SECURE === 'false') return false

	const sslConfig = getSslConfig()
	if (sslConfig && sslConfig.enabled !== false && sslConfig.cert && sslConfig.key) {
		return true
	}

	if (event) {
		// Check direct TLS socket encryption (standalone direct HTTPS)
		const socket = event.node?.req?.socket as any
		if (socket?.encrypted) {
			return true
		}

		// Check proxy forwarding header (if reverse proxy / load balancer is present)
		const proto = getRequestHeader(event, 'x-forwarded-proto')
		if (proto === 'https') {
			return true
		}
	}

	return process.env.NODE_ENV === 'production'
}

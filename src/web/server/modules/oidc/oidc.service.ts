// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import process from 'node:process'
import * as client from 'openid-client'
import * as undici from 'undici'
import { getTlsConnectCacheKey, resolveTlsConnectOptions } from '@@/server/services/ssl.service'
import type { IOidcConfig } from '@@/shared/types/cluster.type'
import type { IOidcTokenSet, IOidcTransaction } from '@@/shared/types/oidc.type'

interface IDiscoveryEntry {
	cacheKey: string
	expiresAt: number
	configuration: client.Configuration
	agent: undici.Agent
	refCount: number
	retired: boolean
}

interface IAcquiredConfiguration {
	configuration: client.Configuration
	release: () => void
}

interface IPendingDiscovery {
	id: number
	cacheKey: string
	promise: Promise<IDiscoveryEntry>
}

const DISCOVERY_CACHE_TTL_MS = 60 * 60 * 1000
export const OIDC_CONNECT_TIMEOUT_MS = 10_000
export const OIDC_HEADERS_TIMEOUT_MS = 15_000
export const OIDC_BODY_TIMEOUT_MS = 15_000

let cachedDiscovery: IDiscoveryEntry | null = null
let pendingDiscovery: IPendingDiscovery | null = null
let discoveryCacheGeneration = 0
const loggedInsecureIssuerWarnings = new Set<string>()

function closeAgentSafely(agent?: undici.Agent): void {
	if (!agent) return
	void agent.close().catch(() => {})
}

function retireDiscoveryEntry(entry: IDiscoveryEntry | null): void {
	if (!entry || entry.retired) return
	entry.retired = true
	if (entry.refCount <= 0) {
		closeAgentSafely(entry.agent)
	}
}

function acquireHandleFromEntry(entry: IDiscoveryEntry): IAcquiredConfiguration {
	entry.refCount += 1
	let released = false
	return {
		configuration: entry.configuration,
		release: () => {
			if (released) return
			released = true
			entry.refCount -= 1
			if (entry.retired && entry.refCount <= 0) {
				closeAgentSafely(entry.agent)
			}
		}
	}
}

function warnInsecureIssuerInProductionOnce(issuerOrigin: string): void {
	if (process.env.NODE_ENV !== 'production') return
	if (loggedInsecureIssuerWarnings.has(issuerOrigin)) return
	loggedInsecureIssuerWarnings.add(issuerOrigin)
	console.warn(
		`[Patrolli-FE OIDC] Insecure HTTP issuer "${issuerOrigin}" is configured in production; use HTTPS in production environments.`
	)
}

export function clearOidcDiscoveryCache(): void {
	discoveryCacheGeneration += 1
	pendingDiscovery = null
	loggedInsecureIssuerWarnings.clear()
	if (cachedDiscovery) {
		const prev = cachedDiscovery
		cachedDiscovery = null
		retireDiscoveryEntry(prev)
	}
}

function buildDiscoveryCacheKey(config: IOidcConfig): string {
	return [
		config.issuer,
		config.client_id,
		config.client_secret || '',
		config.token_endpoint_auth_method || '',
		getTlsConnectCacheKey([config.ca_cert_path])
	].join('|')
}

export function resolveOidcClientAuth(config: IOidcConfig): client.ClientAuth {
	if (!config.client_secret) {
		return client.None()
	}

	if (config.token_endpoint_auth_method === 'client_secret_basic') {
		return client.ClientSecretBasic(config.client_secret)
	}

	if (config.token_endpoint_auth_method === 'client_secret_post') {
		return client.ClientSecretPost(config.client_secret)
	}

	const basicAuth = client.ClientSecretBasic(config.client_secret)
	const postAuth = client.ClientSecretPost(config.client_secret)

	return (as, clientMetadata, body, headers) => {
		const supported = as.token_endpoint_auth_methods_supported
		if (
			Array.isArray(supported) &&
			!supported.includes('client_secret_post') &&
			supported.includes('client_secret_basic')
		) {
			return basicAuth(as, clientMetadata, body, headers)
		}
		return postAuth(as, clientMetadata, body, headers)
	}
}

function createOidcCustomFetch(config: IOidcConfig): {
	customFetch: client.CustomFetch
	agent: undici.Agent
} {
	const tlsConnect = resolveTlsConnectOptions([config.ca_cert_path])
	const agent = new undici.Agent({
		headersTimeout: OIDC_HEADERS_TIMEOUT_MS,
		bodyTimeout: OIDC_BODY_TIMEOUT_MS,
		connect: {
			...tlsConnect,
			timeout: OIDC_CONNECT_TIMEOUT_MS
		}
	})

	const customFetch = ((url, init) => {
		return undici.fetch(url as any, {
			...(init as any),
			dispatcher: agent
		}) as unknown as Promise<Response>
	}) as client.CustomFetch

	return { customFetch, agent }
}

async function runOidcDiscovery(
	config: IOidcConfig,
	cacheKey: string,
	requestId: number
): Promise<IDiscoveryEntry> {
	const issuerUrl = new URL(config.issuer)
	if (issuerUrl.protocol === 'http:') {
		warnInsecureIssuerInProductionOnce(issuerUrl.origin)
	}

	const { customFetch: customFetchFn, agent } = createOidcCustomFetch(config)
	const options: client.DiscoveryRequestOptions = {
		[client.customFetch]: customFetchFn,
		...(issuerUrl.protocol === 'http:' ? { execute: [client.allowInsecureRequests] } : {})
	}
	const clientAuth = resolveOidcClientAuth(config)

	let configuration: client.Configuration
	try {
		configuration = await client.discovery(
			issuerUrl,
			config.client_id,
			undefined,
			clientAuth,
			options
		)
	} catch (error) {
		closeAgentSafely(agent)
		throw error
	}

	const entry: IDiscoveryEntry = {
		cacheKey,
		expiresAt: Date.now() + DISCOVERY_CACHE_TTL_MS,
		configuration,
		agent,
		refCount: 0,
		retired: false
	}

	if (discoveryCacheGeneration === requestId && pendingDiscovery?.id === requestId) {
		const prev = cachedDiscovery
		cachedDiscovery = entry
		retireDiscoveryEntry(prev)
		return entry
	}

	entry.retired = true
	return entry
}

export async function acquireOidcClientConfiguration(
	config: IOidcConfig
): Promise<IAcquiredConfiguration> {
	const cacheKey = buildDiscoveryCacheKey(config)
	if (cachedDiscovery) {
		if (cachedDiscovery.cacheKey === cacheKey && Date.now() < cachedDiscovery.expiresAt) {
			return acquireHandleFromEntry(cachedDiscovery)
		}
		const stale = cachedDiscovery
		cachedDiscovery = null
		retireDiscoveryEntry(stale)
	}

	if (pendingDiscovery && pendingDiscovery.cacheKey === cacheKey) {
		return acquireHandleFromEntry(await pendingDiscovery.promise)
	}

	const requestId = ++discoveryCacheGeneration
	const discoveryPromise = runOidcDiscovery(config, cacheKey, requestId)
	pendingDiscovery = {
		id: requestId,
		cacheKey,
		promise: discoveryPromise
	}

	let resolved: IDiscoveryEntry
	try {
		resolved = await discoveryPromise
	} finally {
		if (pendingDiscovery?.id === requestId) {
			pendingDiscovery = null
		}
	}

	return acquireHandleFromEntry(resolved)
}

export async function createOidcAuthorizationRequest(
	config: IOidcConfig,
	redirectUri: string
): Promise<{ url: string; transaction: IOidcTransaction }> {
	const { configuration, release } = await acquireOidcClientConfiguration(config)
	try {
		const codeVerifier = client.randomPKCECodeVerifier()
		const codeChallenge = await client.calculatePKCECodeChallenge(codeVerifier)
		const state = client.randomState()
		const nonce = client.randomNonce()

		const authorizationUrl = client.buildAuthorizationUrl(configuration, {
			redirect_uri: redirectUri,
			scope: config.scopes,
			code_challenge: codeChallenge,
			code_challenge_method: 'S256',
			state,
			nonce
		})

		return {
			url: authorizationUrl.href,
			transaction: {
				codeVerifier,
				state,
				nonce,
				redirectUri
			}
		}
	} finally {
		release()
	}
}

function toOidcTokenSet(tokens: IOidcTokenSet, fallbackRefreshToken?: string): IOidcTokenSet {
	return {
		access_token: tokens.access_token,
		id_token: tokens.id_token,
		refresh_token: tokens.refresh_token ?? fallbackRefreshToken,
		expires_in: tokens.expires_in,
		token_type: tokens.token_type,
		scope: tokens.scope
	}
}

export async function exchangeOidcAuthorizationCode(
	config: IOidcConfig,
	callbackUrl: URL,
	transaction?: Partial<IOidcTransaction>
): Promise<IOidcTokenSet> {
	const { configuration, release } = await acquireOidcClientConfiguration(config)
	try {
		const tokens = await client.authorizationCodeGrant(configuration, callbackUrl, {
			...(transaction?.codeVerifier ? { pkceCodeVerifier: transaction.codeVerifier } : {}),
			...(transaction?.state ? { expectedState: transaction.state } : {}),
			...(transaction?.nonce ? { expectedNonce: transaction.nonce } : {})
		})

		return toOidcTokenSet(tokens)
	} finally {
		release()
	}
}

export async function refreshOidcTokens(
	config: IOidcConfig,
	refreshToken: string
): Promise<IOidcTokenSet> {
	const { configuration, release } = await acquireOidcClientConfiguration(config)
	try {
		const tokens = await client.refreshTokenGrant(configuration, refreshToken)

		return toOidcTokenSet(tokens, refreshToken)
	} finally {
		release()
	}
}

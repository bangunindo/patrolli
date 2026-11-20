// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import crypto from 'node:crypto'
import process from 'node:process'
import { deleteCookie, getCookie, getRequestProtocol, setCookie, type H3Event } from 'h3'
import { getOidcConfig } from '@@/server/services/cluster.service'
import { isRequestSecure, parseForwardedProto } from '@@/server/services/ssl.service'
import {
	decodeJwtPayload,
	extractRemainingTokenSeconds,
	OIDC_TX_COOKIE_NAME,
	type IAuthTokens
} from '@@/server/utils/auth-cookies'
import type { IOidcConfig, OidcTokenSource } from '@@/shared/types/cluster.type'
import type { IOidcTokenSet, IOidcTransaction } from '@@/shared/types/oidc.type'

export { OIDC_TX_COOKIE_NAME }
const OIDC_TX_MAX_AGE_SECONDS = 300
const OIDC_CALLBACK_PATH = '/auth/oidc/callback'
const EPHEMERAL_TX_SIGNING_KEY = crypto.randomBytes(32)
let hasWarnedEphemeralTxSigningKey = false

export function resetOidcTxSigningKeyWarning(): void {
	hasWarnedEphemeralTxSigningKey = false
}

function isValidHttpUrl(value?: string): boolean {
	if (!value) return false
	try {
		const parsed = new URL(value)
		return parsed.protocol === 'http:' || parsed.protocol === 'https:'
	} catch {
		return false
	}
}

function isValidOidcCallbackUri(value?: string): boolean {
	if (!value) return false
	try {
		const parsed = new URL(value)
		return (
			(parsed.protocol === 'http:' || parsed.protocol === 'https:') &&
			!parsed.username &&
			!parsed.password &&
			!parsed.search &&
			!parsed.hash &&
			parsed.pathname === OIDC_CALLBACK_PATH
		)
	} catch {
		return false
	}
}

function isJwtToken(token?: string): boolean {
	if (!token || typeof token !== 'string') return false
	const parts = token.split('.')
	return parts.length === 3 && decodeJwtPayload(token) !== null
}

export function createOidcConfigErrorPayload(detail: string) {
	return {
		status_code: 500,
		success: false,
		error: {
			title: 'Configuration Error',
			detail
		}
	}
}

export function getValidatedOidcConfig():
	| { config: IOidcConfig; hasRawConfig: true; errorPayload?: undefined }
	| { config?: undefined; hasRawConfig: boolean; errorPayload: Record<string, any> } {
	let oidcConfig: IOidcConfig | null = null
	try {
		oidcConfig = getOidcConfig()
	} catch {
		oidcConfig = null
	}

	if (!oidcConfig || !oidcConfig.issuer || !oidcConfig.client_id) {
		return {
			hasRawConfig: Boolean(oidcConfig),
			errorPayload: createOidcConfigErrorPayload(
				'OIDC configuration is missing or incomplete in config-fe.yaml'
			)
		}
	}

	if (!isValidHttpUrl(oidcConfig.issuer)) {
		return {
			hasRawConfig: true,
			errorPayload: createOidcConfigErrorPayload(
				'OIDC issuer must be a valid http or https URL in config-fe.yaml'
			)
		}
	}

	const scopeTokens = oidcConfig.scopes.split(/\s+/).filter(Boolean)
	if (!scopeTokens.includes('openid')) {
		return {
			hasRawConfig: true,
			errorPayload: createOidcConfigErrorPayload(
				'OIDC scopes must include "openid" in config-fe.yaml'
			)
		}
	}

	if (
		oidcConfig.redirect_uri &&
		oidcConfig.redirect_uri !== OIDC_CALLBACK_PATH &&
		!isValidOidcCallbackUri(oidcConfig.redirect_uri)
	) {
		return {
			hasRawConfig: true,
			errorPayload: createOidcConfigErrorPayload(
				`OIDC redirect_uri must be omitted, "${OIDC_CALLBACK_PATH}", or a valid http(s) URL with path "${OIDC_CALLBACK_PATH}" (without credentials, query, or hash) in config-fe.yaml`
			)
		}
	}

	return { config: oidcConfig, hasRawConfig: true }
}

function resolveRequestProtocol(event: H3Event): 'http' | 'https' {
	return (
		parseForwardedProto(event.node.req.headers['x-forwarded-proto']) ??
		getRequestProtocol(event, { xForwardedProto: false })
	)
}

function parseValidHost(rawHeader: string | string[] | undefined): string | null {
	const rawValue = Array.isArray(rawHeader) ? rawHeader[0] : rawHeader
	if (typeof rawValue !== 'string') return null
	const firstHost = rawValue.split(',')[0]?.trim()
	if (!firstHost || /[\s/@?#\\]/.test(firstHost)) return null
	try {
		const parsed = new URL(`http://${firstHost}`)
		if (
			parsed.username ||
			parsed.password ||
			parsed.pathname !== '/' ||
			parsed.search ||
			parsed.hash
		) {
			return null
		}
		return parsed.host
	} catch {
		return null
	}
}

export function resolveDynamicOidcRedirectUri(
	event: H3Event,
	configuredUri?: string
): string | undefined {
	if (configuredUri) {
		if (isValidOidcCallbackUri(configuredUri)) {
			return configuredUri
		}
		if (configuredUri !== OIDC_CALLBACK_PATH) {
			return undefined
		}
	}

	const host =
		parseValidHost(event.node.req.headers['x-forwarded-host']) ||
		parseValidHost(event.node.req.headers.host)
	if (host) {
		const proto = resolveRequestProtocol(event)
		return `${proto}://${host}${OIDC_CALLBACK_PATH}`
	}
	return undefined
}

export function buildOidcCallbackUrl(event: H3Event, redirectUri: string): URL | null {
	if (!isValidOidcCallbackUri(redirectUri)) {
		return null
	}
	const target = new URL(redirectUri)
	const reqUrl = event.node.req.url || ''
	const queryIndex = reqUrl.indexOf('?')
	if (queryIndex !== -1) {
		target.search = reqUrl.slice(queryIndex)
	}
	return target
}

export function selectUpstreamAuthTokens(
	tokens: IOidcTokenSet,
	tokenSource: OidcTokenSource
): IAuthTokens {
	let selectedBearer: string | undefined
	if (tokenSource === 'id_token') {
		selectedBearer =
			tokens.id_token || (isJwtToken(tokens.access_token) ? tokens.access_token : undefined)
	} else {
		selectedBearer = tokens.access_token || tokens.id_token
	}

	const expiresIn =
		tokenSource === 'id_token' && selectedBearer
			? (extractRemainingTokenSeconds(selectedBearer) ?? tokens.expires_in)
			: tokens.expires_in

	return {
		access_token: selectedBearer,
		refresh_token: tokens.refresh_token,
		id_token: tokens.id_token,
		expires_in: expiresIn
	}
}

function resolveOidcTxSigningKey(config?: IOidcConfig): Buffer {
	const envSecret = process.env.OIDC_COOKIE_SECRET?.trim()
	if (envSecret) {
		return crypto.createHash('sha256').update(envSecret, 'utf-8').digest()
	}

	if (config?.client_secret) {
		return crypto
			.createHash('sha256')
			.update(`oidc_tx:${config.issuer}:${config.client_id}:${config.client_secret}`, 'utf-8')
			.digest()
	}

	if (process.env.NODE_ENV === 'production' && !hasWarnedEphemeralTxSigningKey) {
		hasWarnedEphemeralTxSigningKey = true
		console.warn(
			'[Patrolli-FE OIDC] Using an ephemeral OIDC transaction cookie signing key in production. Set OIDC_COOKIE_SECRET when running public OIDC clients across multiple replicas.'
		)
	}

	return EPHEMERAL_TX_SIGNING_KEY
}

function signOidcTxPayload(encodedPayload: string, config?: IOidcConfig): string {
	return crypto
		.createHmac('sha256', resolveOidcTxSigningKey(config))
		.update(encodedPayload, 'utf-8')
		.digest('base64url')
}

export function encodeSignedOidcTransaction(tx: IOidcTransaction, config?: IOidcConfig): string {
	const payload: IOidcTransaction = {
		...tx,
		expiresAt: tx.expiresAt ?? Date.now() + OIDC_TX_MAX_AGE_SECONDS * 1000
	}
	const encodedPayload = Buffer.from(JSON.stringify(payload), 'utf-8').toString('base64url')
	const signature = signOidcTxPayload(encodedPayload, config)
	return `${encodedPayload}.${signature}`
}

export function setOidcTransactionCookie(
	event: H3Event,
	tx: IOidcTransaction,
	config?: IOidcConfig
): void {
	const signedValue = encodeSignedOidcTransaction(tx, config)
	setCookie(event, OIDC_TX_COOKIE_NAME, signedValue, {
		path: '/',
		httpOnly: true,
		sameSite: 'lax',
		secure: isRequestSecure(event),
		maxAge: OIDC_TX_MAX_AGE_SECONDS
	})
}

function verifyAndParseOidcTransaction(raw: string, config?: IOidcConfig): IOidcTransaction | null {
	const dotIndex = raw.indexOf('.')
	if (dotIndex <= 0 || dotIndex !== raw.lastIndexOf('.')) {
		return null
	}

	const encodedPayload = raw.slice(0, dotIndex)
	const providedSignature = raw.slice(dotIndex + 1)
	if (!providedSignature) return null

	const expectedSignature = signOidcTxPayload(encodedPayload, config)
	const providedBuf = Buffer.from(providedSignature, 'utf-8')
	const expectedBuf = Buffer.from(expectedSignature, 'utf-8')
	if (
		providedBuf.length !== expectedBuf.length ||
		!crypto.timingSafeEqual(providedBuf, expectedBuf)
	) {
		return null
	}

	try {
		const parsed = JSON.parse(Buffer.from(encodedPayload, 'base64url').toString('utf-8'))
		const isValidTransaction =
			parsed &&
			typeof parsed.codeVerifier === 'string' &&
			typeof parsed.state === 'string' &&
			typeof parsed.nonce === 'string' &&
			typeof parsed.redirectUri === 'string' &&
			isValidOidcCallbackUri(parsed.redirectUri) &&
			typeof parsed.expiresAt === 'number' &&
			Number.isFinite(parsed.expiresAt) &&
			Date.now() < parsed.expiresAt

		return isValidTransaction ? (parsed as IOidcTransaction) : null
	} catch {
		return null
	}
}

export function consumeOidcTransactionCookie(
	event: H3Event,
	config?: IOidcConfig
): IOidcTransaction | null {
	const raw = getCookie(event, OIDC_TX_COOKIE_NAME)
	deleteCookie(event, OIDC_TX_COOKIE_NAME, {
		path: '/',
		httpOnly: true,
		sameSite: 'lax',
		secure: isRequestSecure(event)
	})
	if (!raw) return null
	return verifyAndParseOidcTransaction(raw, config)
}

export function formatOidcConnectionError(
	error: any,
	defaultMessage = 'Failed to communicate with OIDC provider'
): string {
	const oauthDetail = error?.error_description || error?.error
	const causeDetail =
		error?.cause?.message ||
		error?.cause?.code ||
		(typeof error?.cause === 'string' ? error.cause : undefined)
	const errMessage = oauthDetail || error?.message || defaultMessage
	return causeDetail && !String(errMessage).includes(String(causeDetail))
		? `${errMessage} (${causeDetail})`
		: String(errMessage)
}

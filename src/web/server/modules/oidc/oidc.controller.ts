// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { H3Event } from 'h3'
import { getQuery, readBody, setResponseHeader, setResponseStatus } from 'h3'
import { establishAuthSession } from '@@/server/modules/auth/auth.controller'
import { renderOidcErrorHtml, renderOidcSuccessHtml } from '@@/server/utils/auth-html'
import type { IOidcPublicConfig } from '@@/shared/types/oidc.type'
import { createOidcAuthorizationRequest, exchangeOidcAuthorizationCode } from './oidc.service'
import {
	buildOidcCallbackUrl,
	consumeOidcTransactionCookie,
	createOidcConfigErrorPayload,
	formatOidcConnectionError,
	getValidatedOidcConfig,
	resolveDynamicOidcRedirectUri,
	selectUpstreamAuthTokens,
	setOidcTransactionCookie
} from './oidc.utils'

function respondOidcHtml(event: H3Event, status: number, html: string): string {
	setResponseStatus(event, status)
	setResponseHeader(event, 'content-type', 'text/html; charset=utf-8')
	return html
}

const OIDC_CLIENT_VALIDATION_ERROR_CODES = new Set([
	'OAUTH_RESPONSE_BODY_ERROR',
	'OAUTH_AUTHORIZATION_RESPONSE_ERROR',
	'OAUTH_JSON_ATTRIBUTE_COMPARISON_FAILED',
	'OAUTH_JWT_CLAIM_COMPARISON_FAILED',
	'OAUTH_JWT_TIMESTAMP_CHECK_FAILED',
	'OAUTH_INVALID_RESPONSE',
	'OAUTH_INVALID_REQUEST'
])

function resolveOidcErrorStatus(err: any): number {
	if (typeof err?.status === 'number' && err.status >= 400 && err.status < 600) {
		return err.status
	}
	if (typeof err?.code === 'string' && OIDC_CLIENT_VALIDATION_ERROR_CODES.has(err.code)) {
		return 400
	}
	return 502
}

const loggedPublicConfigWarnings = new Set<string>()

function warnInvalidOidcConfigOnce(detail: string): void {
	const message = `[Patrolli-FE OIDC] Disabling OIDC login due to invalid config: ${detail}`
	if (loggedPublicConfigWarnings.has(message)) return
	loggedPublicConfigWarnings.add(message)
	console.warn(message)
}

export function handleOidcPublicConfig(event?: H3Event): {
	status_code: number
	success: boolean
	data: IOidcPublicConfig
} {
	if (event) {
		setResponseHeader(event, 'cache-control', 'public, max-age=60')
	}
	const { config, hasRawConfig, errorPayload } = getValidatedOidcConfig()
	if (!config && hasRawConfig && errorPayload?.error?.detail) {
		warnInvalidOidcConfigOnce(String(errorPayload.error.detail))
	}
	return {
		status_code: 200,
		success: true,
		data: {
			enabled: Boolean(config),
			display_name: config?.display_name || 'SSO'
		}
	}
}

export async function handleOidcLogin(event: H3Event) {
	const { config: oidcConfig, errorPayload } = getValidatedOidcConfig()
	if (!oidcConfig) {
		setResponseStatus(event, 500)
		return errorPayload
	}

	const redirectUri = resolveDynamicOidcRedirectUri(event, oidcConfig.redirect_uri)
	if (!redirectUri) {
		setResponseStatus(event, 500)
		return createOidcConfigErrorPayload('Unable to resolve OIDC callback redirect URI')
	}

	try {
		const { url, transaction } = await createOidcAuthorizationRequest(oidcConfig, redirectUri)
		setOidcTransactionCookie(event, transaction, oidcConfig)

		return {
			status_code: 200,
			success: true,
			data: {
				url,
				host: oidcConfig.issuer,
				client_id: oidcConfig.client_id,
				redirect_uri: redirectUri
			}
		}
	} catch (error: any) {
		setResponseStatus(event, 502)
		return {
			status_code: 502,
			success: false,
			error: {
				title: 'Bad Gateway',
				detail: formatOidcConnectionError(error)
			}
		}
	}
}

function getSingleQueryParam(value: unknown): string | undefined {
	if (Array.isArray(value)) {
		const first = value[0]
		return typeof first === 'string' ? first : undefined
	}
	return typeof value === 'string' ? value : undefined
}

export async function handleOidcCallback(event: H3Event) {
	const query = getQuery(event)
	const code = getSingleQueryParam(query.code)
	const error = getSingleQueryParam(query.error_description) || getSingleQueryParam(query.error)

	const { config: oidcConfig, errorPayload } = getValidatedOidcConfig()
	const transaction = consumeOidcTransactionCookie(event, oidcConfig)

	if (error || !code) {
		const errMessage = error || 'Authorization code not found in callback query.'
		return respondOidcHtml(event, 400, renderOidcErrorHtml(errMessage))
	}

	if (!oidcConfig) {
		return respondOidcHtml(event, 500, renderOidcErrorHtml(errorPayload.error.detail))
	}

	const callbackUrl = transaction ? buildOidcCallbackUrl(event, transaction.redirectUri) : null
	if (!transaction || !callbackUrl) {
		return respondOidcHtml(
			event,
			400,
			renderOidcErrorHtml(
				'OIDC login session expired or invalid. Please try signing in again.'
			)
		)
	}

	try {
		const tokenSet = await exchangeOidcAuthorizationCode(oidcConfig, callbackUrl, transaction)
		const authTokens = selectUpstreamAuthTokens(tokenSet, oidcConfig.token_source)

		if (!authTokens.access_token) {
			return respondOidcHtml(
				event,
				400,
				renderOidcErrorHtml('OIDC provider did not return an access or ID token.')
			)
		}

		await establishAuthSession(event, authTokens, 'oidc')
		return respondOidcHtml(event, 200, renderOidcSuccessHtml())
	} catch (err: any) {
		const status = resolveOidcErrorStatus(err)
		return respondOidcHtml(event, status, renderOidcErrorHtml(formatOidcConnectionError(err)))
	}
}

export async function handleLegacyKeycloakCallback(event: H3Event) {
	const body = await readBody(event).catch(() => ({}))
	const code = typeof body?.code === 'string' ? body.code.trim() : ''

	if (!code) {
		setResponseStatus(event, 400)
		return {
			status_code: 400,
			success: false,
			error: {
				title: 'Bad Request',
				detail: 'Authorization code is required'
			}
		}
	}

	const { config: oidcConfig, errorPayload } = getValidatedOidcConfig()
	if (!oidcConfig) {
		setResponseStatus(event, 500)
		return errorPayload
	}

	const transaction = consumeOidcTransactionCookie(event, oidcConfig)
	const resolvedRedirectUri =
		transaction?.redirectUri ||
		(typeof body?.redirect_uri === 'string' && body.redirect_uri.trim()) ||
		resolveDynamicOidcRedirectUri(event, oidcConfig.redirect_uri) ||
		'https://localhost:3000/auth/oidc/callback'

	let callbackUrl: URL
	try {
		callbackUrl = new URL(resolvedRedirectUri)
		callbackUrl.searchParams.set('code', code)
		const stateParam =
			(typeof body?.state === 'string' && body.state.trim()) || transaction?.state
		if (stateParam) {
			callbackUrl.searchParams.set('state', stateParam)
		}
	} catch {
		setResponseStatus(event, 400)
		return {
			status_code: 400,
			success: false,
			error: {
				title: 'Bad Request',
				detail: 'Invalid redirect_uri provided for callback'
			}
		}
	}

	try {
		const tokenSet = await exchangeOidcAuthorizationCode(
			oidcConfig,
			callbackUrl,
			transaction ?? undefined
		)
		const authTokens = selectUpstreamAuthTokens(tokenSet, oidcConfig.token_source)

		if (!authTokens.access_token) {
			setResponseStatus(event, 400)
			return {
				status_code: 400,
				success: false,
				error: {
					title: 'OIDC Authentication Error',
					detail: 'OIDC provider did not return an access or ID token'
				}
			}
		}

		const sessionData = await establishAuthSession(event, authTokens, 'oidc')
		return {
			status_code: 200,
			success: true,
			data: sessionData
		}
	} catch (error: any) {
		const status = resolveOidcErrorStatus(error)
		const isClientError = status >= 400 && status < 500
		setResponseStatus(event, status)
		return {
			status_code: status,
			success: false,
			error: {
				title: isClientError ? 'OIDC Authentication Error' : 'Bad Gateway',
				detail: formatOidcConnectionError(error)
			}
		}
	}
}

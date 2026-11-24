// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { computed, ref } from 'vue'
import { defineStore } from 'pinia'
import authApi from '@/app/api/auth.api'
import clustersApi, { formatClusterOptions } from '@/app/api/clusters.api'
import cookieHelper from '@/app/helpers/cookie.helper'
import { useAppStore } from '@/stores/app.store'
import type { AuthProvider, IAuthUser, ILoginBody } from '@@/shared/types/auth.type'
import { normalizeAuthProvider } from '@@/shared/utils/auth'

const PREFLIGHT_REFRESH_LEEWAY_MS = 30 * 1000
export const AUTH_REFRESH_LOCK_NAME = 'patrolli-auth-refresh'

export const useAuthStore = defineStore(
	'auth',
	() => {
		let activeRefreshPromise: Promise<boolean> | null = null
		let activeUserFetchPromise: Promise<IAuthUser | null> | null = null
		let activeLogoutPromise: Promise<void> | null = null
		const getAuthCookie = () => {
			if (typeof useCookie === 'function') {
				return useCookie<boolean>('is_authenticated', {
					default: () => false,
					sameSite: 'lax',
					path: '/'
				})
			}
			const val = cookieHelper.get('is_authenticated') === 'true'
			return ref(val)
		}

		const getInitialAuthProvider = (): AuthProvider | null => {
			return normalizeAuthProvider(cookieHelper.get('auth_provider'))
		}

		const getNumericCookie = (name: string): number | null => {
			const raw = cookieHelper.get(name)
			if (!raw) return null
			const parsed = Number(raw)
			return Number.isFinite(parsed) && parsed > 0 ? parsed : null
		}

		const authCookie = getAuthCookie()
		const authenticated = ref(Boolean(authCookie.value))
		const authProvider = ref<AuthProvider | null>(getInitialAuthProvider())
		const isClusterReadonly = computed(() => authProvider.value === 'local')
		const user = ref<IAuthUser | null>(null)
		const userMenus = ref()
		const error = ref<string | null>(null)
		const lastRefreshedAt = ref<number>(getNumericCookie('auth_refreshed_at') ?? Date.now())
		const tokenExpiresAt = ref<number | null>(getNumericCookie('access_token_exp'))

		function hasActiveSession(): boolean {
			return (
				authenticated.value ||
				Boolean(authCookie.value) ||
				cookieHelper.get('is_authenticated') === 'true'
			)
		}

		function isAccessTokenExpiringSoon(leewayMs = PREFLIGHT_REFRESH_LEEWAY_MS): boolean {
			const expAt = getNumericCookie('access_token_exp') ?? tokenExpiresAt.value
			if (expAt === null) return false
			return Date.now() >= expAt - leewayMs
		}

		function isSessionInvalidError(err: any): boolean {
			const status = err?.status ?? err?.response?.status
			return (
				status === 401 ||
				status === 403 ||
				err?.data?.is_session_invalid === true ||
				err?.data?.data?.error === 'invalid_grant'
			)
		}

		function resetAuthState() {
			authenticated.value = false
			authCookie.value = false
			authProvider.value = null
			tokenExpiresAt.value = null
		}

		function applyAuthenticatedSession(options: {
			user?: IAuthUser | null
			provider?: AuthProvider | 'keycloak' | null
			expiresIn?: number
			updateRefreshedAt?: boolean
		}) {
			authenticated.value = true
			authCookie.value = true
			if (options.user !== undefined) {
				user.value = options.user
			}
			const normalizedProvider = normalizeAuthProvider(options.provider)
			if (normalizedProvider) {
				authProvider.value = normalizedProvider
			}
			if (
				typeof options.expiresIn === 'number' &&
				Number.isFinite(options.expiresIn) &&
				options.expiresIn > 0
			) {
				tokenExpiresAt.value = Date.now() + options.expiresIn * 1000
			} else {
				const cookieExp = getNumericCookie('access_token_exp')
				if (cookieExp !== null) {
					tokenExpiresAt.value = cookieExp
				}
			}
			if (options.updateRefreshedAt) {
				lastRefreshedAt.value = Date.now()
			}
		}

		async function fetchCurrentUser(): Promise<IAuthUser | null> {
			if (activeUserFetchPromise) return activeUserFetchPromise

			activeUserFetchPromise = (async () => {
				try {
					const response = await authApi.me()
					if (response?.success && response.data?.user) {
						applyAuthenticatedSession({
							user: response.data.user,
							provider: response.data.auth_provider,
							expiresIn: response.data.expires_in
						})
						return user.value
					}
					return null
				} catch (err: any) {
					if (isSessionInvalidError(err) && hasActiveSession()) {
						await logout()
					}
					return null
				} finally {
					activeUserFetchPromise = null
				}
			})()

			return activeUserFetchPromise
		}

		async function ensureCurrentUser(): Promise<IAuthUser | null> {
			if (!hasActiveSession()) return null
			if (user.value) return user.value
			return fetchCurrentUser()
		}

		async function login(payload: ILoginBody, clusterURL?: string) {
			try {
				const clusterURLs = clusterURL ? [clusterURL] : undefined
				const response = await authApi.login(payload, clusterURLs)
				if (response && response.success) {
					applyAuthenticatedSession({
						user: response.data?.user,
						provider: 'local',
						expiresIn: response.data?.expires_in,
						updateRefreshedAt: true
					})

					if (!response.data?.user) {
						await fetchCurrentUser()
					}

					if (clusterURL) {
						try {
							const appStore = useAppStore()
							appStore.activeClusterURL = clusterURL
						} catch {
							// App store not initialized
						}
					}

					return response
				} else {
					resetAuthState()
					throw new Error('Login failed')
				}
			} catch (err: any) {
				error.value = err.message || 'Failed to login'
				resetAuthState()
				throw err
			}
		}

		async function refreshAccessToken(options?: {
			skipIfFresh?: boolean
			skipIfRefreshedWithinMs?: number
		}): Promise<boolean> {
			if (activeRefreshPromise) {
				return activeRefreshPromise
			}

			activeRefreshPromise = (async () => {
				const runRefresh = async (): Promise<boolean> => {
					const cookieExp = getNumericCookie('access_token_exp')
					const cookieRefreshedAt = getNumericCookie('auth_refreshed_at')

					if (
						options?.skipIfRefreshedWithinMs &&
						cookieRefreshedAt !== null &&
						Date.now() - cookieRefreshedAt < options.skipIfRefreshedWithinMs
					) {
						if (cookieExp !== null) {
							tokenExpiresAt.value = cookieExp
						}
						lastRefreshedAt.value = cookieRefreshedAt
						return true
					}

					if (options?.skipIfFresh) {
						if (
							cookieExp !== null &&
							Date.now() < cookieExp - PREFLIGHT_REFRESH_LEEWAY_MS
						) {
							tokenExpiresAt.value = cookieExp
							lastRefreshedAt.value = cookieRefreshedAt ?? Date.now()
							return true
						}
					}

					const response = await authApi.refreshToken()

					if (response && response.success) {
						const provider = normalizeAuthProvider(cookieHelper.get('auth_provider'))
						applyAuthenticatedSession({
							user: response.data?.user,
							provider,
							expiresIn: response.data?.expires_in,
							updateRefreshedAt: true
						})
						return true
					}

					return false
				}

				try {
					const locks = typeof navigator !== 'undefined' ? navigator.locks : undefined
					return typeof locks?.request === 'function'
						? await locks.request(AUTH_REFRESH_LOCK_NAME, () => runRefresh())
						: await runRefresh()
				} catch (err: any) {
					console.error('Failed to refresh token in Nitro:', err)
					if (isSessionInvalidError(err)) {
						await logout()
					}
					return false
				} finally {
					activeRefreshPromise = null
				}
			})()

			return activeRefreshPromise
		}

		async function ensureValidAccessToken(): Promise<boolean> {
			if (!hasActiveSession()) return false
			if (!isAccessTokenExpiringSoon()) return true
			return refreshAccessToken({ skipIfFresh: true })
		}

		async function logout() {
			if (activeLogoutPromise) {
				return activeLogoutPromise
			}

			activeLogoutPromise = (async () => {
				try {
					await authApi.logout()
				} catch (err) {
					console.error(err)
				} finally {
					cookieHelper.clearAll()
					cookieHelper.remove('user_clusters')
					resetAuthState()
					user.value = null
					userMenus.value = null

					try {
						const appStore = useAppStore()
						appStore.notificationItems = []
						appStore.activeClusterURL = null
					} catch {
						// ignore
					}

					if (typeof navigateTo === 'function') {
						await navigateTo('/account/login')
					}
					activeLogoutPromise = null
				}
			})()

			return activeLogoutPromise
		}

		async function refreshUserClusters() {
			try {
				cookieHelper.remove('user_clusters')
				const response = await clustersApi.getClusters({ refresh: true })
				if (response && response.data) {
					const formatted = formatClusterOptions(response.data)

					cookieHelper.set('user_clusters', JSON.stringify(formatted), 7)

					if (typeof window !== 'undefined') {
						window.__APP_CONFIG__ = {
							...window.__APP_CONFIG__,
							API_URLS: formatted.map((c) => c.name)
						}
					}

					const appStore = useAppStore()
					appStore.setAvailableClusters(formatted)

					if (formatted.length === 1 && formatted[0]?.value) {
						appStore.activeClusterURL = formatted[0].value
					} else if (appStore.activeClusterURL) {
						const valid = formatted.some(
							(c: any) => c.value === appStore.activeClusterURL
						)
						if (!valid) {
							appStore.activeClusterURL = null
						}
					}
				}
			} catch (err) {
				console.error('Failed to refresh user clusters:', err)
			}
		}

		return {
			authenticated,
			authProvider,
			isClusterReadonly,
			user,
			userMenus,
			error,
			lastRefreshedAt,
			tokenExpiresAt,

			hasActiveSession,
			isAccessTokenExpiringSoon,
			ensureValidAccessToken,
			login,
			fetchCurrentUser,
			ensureCurrentUser,
			refreshAccessToken,
			refreshUserClusters,
			logout
		}
	},
	{
		persist: {
			omit: ['user', 'tokenExpiresAt', 'lastRefreshedAt'],
			afterHydrate: (ctx) => {
				ctx.store.authProvider = normalizeAuthProvider(ctx.store.authProvider)
			}
		}
	}
)

// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { computed, ref } from 'vue'
import { defineStore } from 'pinia'
import authApi from '@/app/api/auth.api'
import cookieHelper from '@/app/helpers/cookie.helper'
import tokenHelper from '@/app/helpers/token.helper'
import { useAppStore } from '@/stores/app.store'
import type { ILoginBody } from '@@/shared/types/auth.type'

export const useAuthStore = defineStore(
	'auth',
	() => {
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

		const getInitialAuthProvider = (): 'local' | 'keycloak' | null => {
			const cookieVal = cookieHelper.get('auth_provider')
			if (cookieVal === 'local' || cookieVal === 'keycloak') return cookieVal
			return null
		}

		const authCookie = getAuthCookie()
		const authenticated = ref(Boolean(authCookie.value))
		const authProvider = ref<'local' | 'keycloak' | null>(getInitialAuthProvider())
		const isClusterReadonly = computed(() => authProvider.value === 'local')
		const userRoles = ref()
		const userPermissions = ref()
		const userMenus = ref()
		const error = ref<string | null>(null)

		async function login(payload: ILoginBody, clusterURL?: string) {
			try {
				const clusterURLs = clusterURL ? [clusterURL] : undefined
				const response = await authApi.login(payload, clusterURLs)
				if (response && response.success) {
					authenticated.value = true
					authCookie.value = true
					authProvider.value = 'local'
					cookieHelper.set('auth_provider', 'local', 7)
					cookieHelper.set('is_authenticated', 'true', 1)

					if (response.data?.access_token) {
						tokenHelper.setTokens({
							access_token: response.data.access_token,
							refresh_token: response.data.refresh_token,
							id_token: response.data.id_token
						})
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
					authenticated.value = false
					authCookie.value = false
					authProvider.value = null
					throw new Error('Login failed')
				}
			} catch (err: any) {
				error.value = err.message || 'Failed to login'
				authenticated.value = false
				authCookie.value = false
				authProvider.value = null
				throw err
			}
		}

		async function refreshAccessToken(): Promise<string | null> {
			try {
				// Nitro BFF handles token refresh using HTTP cookies
				const response = await $fetch<{ success: boolean; data?: any }>(
					'/api/auth/refresh',
					{
						method: 'POST',
						credentials: 'include'
					}
				)

				if (response && response.success && response.data?.access_token) {
					const data = response.data
					tokenHelper.setTokens({
						access_token: data.access_token,
						refresh_token: data.refresh_token,
						id_token: data.id_token
					})
					authenticated.value = true
					authCookie.value = true
					const provider = cookieHelper.get('auth_provider') as
						| 'local'
						| 'keycloak'
						| null
					if (provider) {
						authProvider.value = provider
					}
					return data.access_token
				}

				throw new Error('No access token returned from Nitro refresh endpoint')
			} catch (err) {
				console.error('Failed to refresh token in Nitro:', err)
				await logout()
				return null
			}
		}

		async function logout() {
			try {
				await authApi.logout()
			} catch (err) {
				console.error(err)
			} finally {
				cookieHelper.clearAll()
				cookieHelper.remove('user_clusters')
				tokenHelper.clearTokens()
				authenticated.value = false
				authCookie.value = false
				authProvider.value = null
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
			}
		}

		async function refreshUserClusters() {
			try {
				cookieHelper.remove('user_clusters')
				const response = await $fetch<{ success: boolean; data: any }>(
					'/api/clusters?refresh=true'
				)
				if (response && response.data) {
					const rawList = Array.isArray(response.data) ? response.data : []
					const formatted = rawList.map((item: any) => ({
						alias: item.key || item.name || item.value,
						name: item.name || item.key || item.value,
						label: item.label || item.name || item.key || item.value,
						value: item.value || item.clientPath || item.url || item.name
					}))

					cookieHelper.set('user_clusters', JSON.stringify(formatted), 7)

					if (typeof window !== 'undefined') {
						window.__APP_CONFIG__ = {
							...window.__APP_CONFIG__,
							API_URLS: formatted.map((c: any) => c.name)
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
			userRoles,
			userPermissions,
			userMenus,
			error,

			login,
			refreshAccessToken,
			refreshUserClusters,
			logout
		}
	},
	{ persist: true }
)

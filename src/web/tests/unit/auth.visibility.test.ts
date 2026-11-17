// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { useAuthStore } from '@/stores/auth.store'

vi.hoisted(() => {
	;(globalThis as any).defineNuxtPlugin = (fn: any) => fn
})

describe('auth-visibility keep-alive plugin', () => {
	beforeEach(() => {
		vi.useFakeTimers()
		setActivePinia(createPinia())
		Object.defineProperty(document, 'cookie', {
			writable: true,
			value: ''
		})
	})

	afterEach(() => {
		vi.useRealTimers()
		vi.restoreAllMocks()
		document.cookie = ''
	})

	it('refreshes every 8 minutes when authenticated and skips when unauthenticated', async () => {
		const { KEEP_ALIVE_INTERVAL_MS, setupAuthKeepAlive } =
			await import('@/plugins/auth-visibility.client')
		const authStore = useAuthStore()
		const refreshSpy = vi
			.spyOn(authStore, 'refreshAccessToken')
			.mockImplementation(async () => {
				authStore.lastRefreshedAt = Date.now()
				return true
			})

		authStore.authenticated = false
		const cleanup = setupAuthKeepAlive()

		await vi.advanceTimersByTimeAsync(KEEP_ALIVE_INTERVAL_MS)
		expect(refreshSpy).not.toHaveBeenCalled()

		authStore.lastRefreshedAt = Date.now()
		authStore.authenticated = true
		await Promise.resolve()

		await vi.advanceTimersByTimeAsync(KEEP_ALIVE_INTERVAL_MS)
		expect(refreshSpy).toHaveBeenCalledTimes(1)

		await vi.advanceTimersByTimeAsync(KEEP_ALIVE_INTERVAL_MS)
		expect(refreshSpy).toHaveBeenCalledTimes(2)

		authStore.authenticated = false
		await Promise.resolve()
		await vi.advanceTimersByTimeAsync(KEEP_ALIVE_INTERVAL_MS)
		expect(refreshSpy).toHaveBeenCalledTimes(2)

		cleanup()
	})

	it('refreshes when is_authenticated cookie is true even if store authenticated is false', async () => {
		const { KEEP_ALIVE_INTERVAL_MS, setupAuthKeepAlive } =
			await import('@/plugins/auth-visibility.client')
		const authStore = useAuthStore()
		authStore.authenticated = false
		document.cookie = 'is_authenticated=true; path=/'
		const refreshSpy = vi.spyOn(authStore, 'refreshAccessToken').mockResolvedValue(true)

		const cleanup = setupAuthKeepAlive()

		await vi.advanceTimersByTimeAsync(KEEP_ALIVE_INTERVAL_MS)
		expect(refreshSpy).toHaveBeenCalledTimes(1)

		cleanup()
	})

	it('refreshes on visibilitychange when tab becomes visible after >= 5 minutes', async () => {
		const { VISIBILITY_MIN_INTERVAL_MS, setupAuthKeepAlive } =
			await import('@/plugins/auth-visibility.client')
		const authStore = useAuthStore()
		authStore.authenticated = true
		const refreshSpy = vi.spyOn(authStore, 'refreshAccessToken').mockResolvedValue(true)

		const cleanup = setupAuthKeepAlive()

		Object.defineProperty(document, 'visibilityState', {
			configurable: true,
			value: 'visible'
		})

		document.dispatchEvent(new Event('visibilitychange'))
		expect(refreshSpy).not.toHaveBeenCalled()

		await vi.advanceTimersByTimeAsync(VISIBILITY_MIN_INTERVAL_MS)
		document.dispatchEvent(new Event('visibilitychange'))
		expect(refreshSpy).toHaveBeenCalledTimes(1)

		cleanup()
	})

	it('resets 8-minute schedule and skips visibility refresh when authStore.lastRefreshedAt is updated externally', async () => {
		const { KEEP_ALIVE_INTERVAL_MS, VISIBILITY_MIN_INTERVAL_MS, setupAuthKeepAlive } =
			await import('@/plugins/auth-visibility.client')
		const authStore = useAuthStore()
		authStore.authenticated = true
		const refreshSpy = vi.spyOn(authStore, 'refreshAccessToken').mockResolvedValue(true)

		const cleanup = setupAuthKeepAlive()

		// Advance 6 minutes (not due yet), then simulate an external 401 refresh
		await vi.advanceTimersByTimeAsync(6 * 60 * 1000)
		expect(refreshSpy).not.toHaveBeenCalled()
		authStore.lastRefreshedAt = Date.now()
		await Promise.resolve()

		// Advancing another 3 minutes (9m total, 3m since lastRefreshedAt) should NOT trigger refresh or visibility refresh
		await vi.advanceTimersByTimeAsync(3 * 60 * 1000)
		Object.defineProperty(document, 'visibilityState', {
			configurable: true,
			value: 'visible'
		})
		document.dispatchEvent(new Event('visibilitychange'))
		expect(refreshSpy).not.toHaveBeenCalled()

		// Advancing remaining 5 minutes (8m since lastRefreshedAt) triggers 1 refresh
		await vi.advanceTimersByTimeAsync(VISIBILITY_MIN_INTERVAL_MS)
		expect(refreshSpy).toHaveBeenCalledTimes(1)
		expect(KEEP_ALIVE_INTERVAL_MS).toBe(8 * 60 * 1000)

		cleanup()
	})

	it('ensureCurrentUser fetches user when is_authenticated cookie is true even if store authenticated is false', async () => {
		const authApi = (await import('@/app/api/auth.api')).default
		const authStore = useAuthStore()
		authStore.authenticated = false
		document.cookie = 'is_authenticated=true; path=/'

		const meSpy = vi.spyOn(authApi, 'me').mockResolvedValueOnce({
			success: true,
			data: {
				user: { preferred_username: 'cookie-user', username: 'cookie-user' },
				auth_provider: 'keycloak'
			}
		})

		const user = await authStore.ensureCurrentUser()
		expect(meSpy).toHaveBeenCalledTimes(1)
		expect(user?.preferred_username).toBe('cookie-user')
		expect(authStore.authenticated).toBe(true)
	})

	it('omits user from localStorage persistence so stale user claims are not hydrated across reloads', async () => {
		const { createApp } = await import('vue')
		const { createPersistedState } = await import('pinia-plugin-persistedstate')
		const app = createApp({})
		const pinia = createPinia()
		pinia.use(createPersistedState({ storage: localStorage }))
		app.use(pinia)
		setActivePinia(pinia)

		localStorage.setItem(
			'auth',
			JSON.stringify({
				authenticated: true,
				authProvider: 'keycloak',
				user: { preferred_username: 'stale-localstorage-user', username: 'stale' }
			})
		)
		document.cookie = 'is_authenticated=true; path=/'

		const authApi = (await import('@/app/api/auth.api')).default
		const meSpy = vi.spyOn(authApi, 'me').mockResolvedValueOnce({
			success: true,
			data: {
				user: { preferred_username: 'fresh-oidc-user', username: 'fresh-oidc-user' },
				auth_provider: 'keycloak'
			}
		})

		const authStore = useAuthStore()
		// Even though localStorage contained 'user', omit: ['user'] prevents it from hydrating into authStore.user
		expect(authStore.user).toBeNull()

		const resolved = await authStore.ensureCurrentUser()
		expect(meSpy).toHaveBeenCalledTimes(1)
		expect(resolved?.preferred_username).toBe('fresh-oidc-user')
		;(authStore as any).$persist?.()
		const persistedRaw = JSON.parse(localStorage.getItem('auth') || '{}')
		expect(persistedRaw.user).toBeUndefined()
		localStorage.removeItem('auth')
	})

	it('fetchCurrentUser deduplicates concurrent calls (single-flight)', async () => {
		const authApi = (await import('@/app/api/auth.api')).default
		const authStore = useAuthStore()
		let resolveMe!: (val: any) => void
		const meSpy = vi.spyOn(authApi, 'me').mockImplementation(
			() =>
				new Promise((resolve) => {
					resolveMe = resolve
				})
		)

		const p1 = authStore.fetchCurrentUser()
		const p2 = authStore.fetchCurrentUser()
		resolveMe({
			success: true,
			data: {
				user: { preferred_username: 'dedup-user', username: 'dedup-user' },
				auth_provider: 'local'
			}
		})
		const [u1, u2] = await Promise.all([p1, p2])

		expect(meSpy).toHaveBeenCalledTimes(1)
		expect(u1?.preferred_username).toBe('dedup-user')
		expect(u2?.preferred_username).toBe('dedup-user')
	})

	it('fetchCurrentUser triggers logout when /api/auth/me rejects with 401 and does not logout on 502', async () => {
		const authApi = (await import('@/app/api/auth.api')).default
		const authStore = useAuthStore()
		authStore.authenticated = true
		const logoutApiSpy = vi.spyOn(authApi, 'logout').mockResolvedValue({ success: true } as any)

		vi.spyOn(authApi, 'me').mockRejectedValueOnce({ status: 502 })
		const res502 = await authStore.fetchCurrentUser()
		expect(res502).toBeNull()
		expect(logoutApiSpy).not.toHaveBeenCalled()
		expect(authStore.authenticated).toBe(true)

		vi.spyOn(authApi, 'me').mockRejectedValueOnce({ status: 401 })
		const res401 = await authStore.fetchCurrentUser()
		expect(res401).toBeNull()
		expect(logoutApiSpy).toHaveBeenCalledTimes(1)
		expect(authStore.authenticated).toBe(false)
	})

	it('skips keep-alive HTTP refresh when another tab recently updated auth_refreshed_at cookie', async () => {
		const { KEEP_ALIVE_INTERVAL_MS, setupAuthKeepAlive } =
			await import('@/plugins/auth-visibility.client')
		const authApi = (await import('@/app/api/auth.api')).default
		const authStore = useAuthStore()
		authStore.authenticated = true

		const refreshApiSpy = vi.spyOn(authApi, 'refreshToken').mockResolvedValue({
			status_code: 200,
			success: true,
			data: { expires_in: 900 }
		})

		const cleanup = setupAuthKeepAlive()

		// Advance almost 8 minutes, then simulate another tab refreshing at 7m50s
		await vi.advanceTimersByTimeAsync(KEEP_ALIVE_INTERVAL_MS - 10_000)
		document.cookie = `is_authenticated=true; auth_refreshed_at=${Date.now()}; access_token_exp=${Date.now() + 900_000}`

		await vi.advanceTimersByTimeAsync(10_000)
		expect(refreshApiSpy).not.toHaveBeenCalled()

		cleanup()
	})

	it('does not advance lastRefreshedAt or reschedule keep-alive when refresh logs the user out', async () => {
		const { KEEP_ALIVE_INTERVAL_MS, setupAuthKeepAlive } =
			await import('@/plugins/auth-visibility.client')
		const authStore = useAuthStore()
		const initialRefreshedAt = Date.now()
		authStore.lastRefreshedAt = initialRefreshedAt
		authStore.authenticated = true

		const refreshSpy = vi
			.spyOn(authStore, 'refreshAccessToken')
			.mockImplementation(async () => {
				authStore.authenticated = false
				return false
			})

		const cleanup = setupAuthKeepAlive()

		await vi.advanceTimersByTimeAsync(KEEP_ALIVE_INTERVAL_MS)
		expect(refreshSpy).toHaveBeenCalledTimes(1)
		expect(authStore.lastRefreshedAt).toBe(initialRefreshedAt)

		await vi.advanceTimersByTimeAsync(KEEP_ALIVE_INTERVAL_MS)
		expect(refreshSpy).toHaveBeenCalledTimes(1)

		cleanup()
	})
})

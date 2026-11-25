// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { watch } from 'vue'
import { useAuthStore } from '@/stores/auth.store'

export const KEEP_ALIVE_INTERVAL_MS = 8 * 60 * 1000
export const VISIBILITY_MIN_INTERVAL_MS = 5 * 60 * 1000

export function setupAuthKeepAlive() {
	const authStore = useAuthStore()
	let timeoutId: ReturnType<typeof setTimeout> | null = null
	let isDisposed = false

	const clearScheduled = () => {
		if (timeoutId !== null) {
			clearTimeout(timeoutId)
			timeoutId = null
		}
	}

	const scheduleNext = () => {
		clearScheduled()
		if (isDisposed || !authStore.hasActiveSession()) return
		const elapsed = Math.max(0, Date.now() - authStore.lastRefreshedAt)
		const delay = Math.max(0, KEEP_ALIVE_INTERVAL_MS - elapsed)
		timeoutId = setTimeout(() => {
			void refreshIfDue(KEEP_ALIVE_INTERVAL_MS)
		}, delay)
	}

	const refreshIfDue = async (minIntervalMs: number) => {
		if (!authStore.hasActiveSession()) {
			clearScheduled()
			return
		}

		const now = Date.now()
		if (now - authStore.lastRefreshedAt < minIntervalMs) {
			scheduleNext()
			return
		}

		await authStore.refreshAccessToken({ skipIfRefreshedWithinMs: minIntervalMs })
		if (!authStore.hasActiveSession()) {
			clearScheduled()
			return
		}
		// Always advance/reschedule even if refresh failed (e.g. transient 502) to avoid tight retry loops;
		// updating lastRefreshedAt triggers stopWatch -> scheduleNext(), so only schedule directly if unchanged
		if (Date.now() - authStore.lastRefreshedAt >= minIntervalMs) {
			authStore.lastRefreshedAt = Date.now()
		} else {
			scheduleNext()
		}
	}

	const handleVisibilityChange = () => {
		if (document.visibilityState !== 'visible') return
		void refreshIfDue(VISIBILITY_MIN_INTERVAL_MS)
	}

	const stopWatch = watch(
		() => [authStore.lastRefreshedAt, authStore.authenticated],
		() => {
			scheduleNext()
		}
	)

	scheduleNext()
	document.addEventListener('visibilitychange', handleVisibilityChange)

	return () => {
		isDisposed = true
		clearScheduled()
		stopWatch()
		document.removeEventListener('visibilitychange', handleVisibilityChange)
	}
}

export default defineNuxtPlugin(() => {
	if (!import.meta.client) return
	setupAuthKeepAlive()
})

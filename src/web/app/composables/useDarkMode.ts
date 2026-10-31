// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { ref, onMounted, onUnmounted } from 'vue'

export function useDarkMode() {
	const isDarkMode = ref(false)

	// Memeriksa preferensi dark mode
	const checkDarkMode = () => {
		const mediaQuery = window.matchMedia('(prefers-color-scheme: dark)')
		isDarkMode.value = mediaQuery.matches
	}

	const mediaQueryListener = (event) => {
		isDarkMode.value = event.matches
	}

	onMounted(() => {
		checkDarkMode()

		const mediaQuery = window.matchMedia('(prefers-color-scheme: dark)')
		mediaQuery.addEventListener('change', mediaQueryListener)
	})

	onUnmounted(() => {
		const mediaQuery = window.matchMedia('(prefers-color-scheme: dark)')
		mediaQuery.removeEventListener('change', mediaQueryListener)
	})

	return isDarkMode
}

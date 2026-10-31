// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import piniaPluginPersistedstate from 'pinia-plugin-persistedstate'

export default defineNuxtPlugin((nuxtApp) => {
	if (nuxtApp.$pinia) {
		;(nuxtApp.$pinia as any).use(piniaPluginPersistedstate)
	}
})

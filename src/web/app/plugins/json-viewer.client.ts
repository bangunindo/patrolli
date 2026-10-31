// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import '@@/shared/types/array.type'
import '@@/shared/types/string.type'
import { JsonViewer } from 'vue3-json-viewer'

export default defineNuxtPlugin((nuxtApp) => {
	nuxtApp.vueApp.component('JsonViewer', JsonViewer)
})

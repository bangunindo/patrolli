// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { defineEventHandler } from 'h3'
import { handleProxyGet } from '@@/server/modules/proxy/proxy.controller'

export default defineEventHandler((event) => {
	if (!event.context) event.context = {} as any
	if (!event.context.params) event.context.params = {}
	if (!event.context.params.path) event.context.params.path = 'users'
	return handleProxyGet(event)
})

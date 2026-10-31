// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { defineEventHandler } from 'h3'
import { handleOidcCallback } from '@@/server/modules/auth/auth.controller'

export default defineEventHandler((event) => {
	return handleOidcCallback(event)
})

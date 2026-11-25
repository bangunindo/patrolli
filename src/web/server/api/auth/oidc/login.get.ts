// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { defineEventHandler } from 'h3'
import { handleOidcLogin } from '@@/server/modules/oidc/oidc.controller'

export default defineEventHandler((event) => {
	return handleOidcLogin(event)
})

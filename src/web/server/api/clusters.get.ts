// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { defineEventHandler } from 'h3'
import { handleGetClusters } from '@@/server/modules/clusters/clusters.controller'

export default defineEventHandler(async (event) => {
	return await handleGetClusters(event)
})

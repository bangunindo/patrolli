// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { v7 as uuidv7 } from 'uuid'

export function generateUuidV7(): string {
	return uuidv7()
}

export default {
	generateUuidV7
}

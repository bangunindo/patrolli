// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { AuthProvider } from '@@/shared/types/auth.type'

export function normalizeAuthProvider(value?: string | null): AuthProvider | null {
	if (value === 'local' || value === 'oidc') return value
	if (value === 'keycloak') return 'oidc'
	return null
}

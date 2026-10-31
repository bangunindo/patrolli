// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { UiHint } from '@@/shared/types/dynamic-form.type'

export const userHints: Record<string, UiHint> = {
	display_name: {
		order: 1,
		group: 'General Info',
		groupVariant: 'section',
		label: 'Full Name',
		placeholder: 'Enter full name (e.g. Bianca Vance)...'
	},
	username: {
		order: 2,
		group: 'General Info',
		groupVariant: 'section',
		label: 'Username',
		placeholder: 'Enter username (e.g. biancavance)...'
	},
	email: {
		order: 3,
		group: 'General Info',
		groupVariant: 'section',
		label: 'Email',
		placeholder: 'user@patrolli.local'
	},
	allow_sso: {
		order: 4,
		group: 'Security Settings',
		groupVariant: 'section',
		label: 'Allow SSO',
		defaultValue: false
	},
	password: {
		order: 5,
		group: 'Security Settings',
		groupVariant: 'section',
		label: 'Password',
		widget: 'password',
		sensitive: true,
		placeholder: 'Enter password...'
	},
	adm_status: {
		order: 6,
		group: 'Security Settings',
		groupVariant: 'section',
		label: 'Status',
		widget: 'switch',
		defaultValue: 1
	}
}

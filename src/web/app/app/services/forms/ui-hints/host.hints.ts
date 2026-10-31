// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { UiHint } from '@@/shared/types/dynamic-form.type'

export const hostHints: Record<string, UiHint> = {
	name: { order: 1, placeholder: 'Name of host' },
	adm_status: { order: 2, label: 'Status', widget: 'switch' },
	labels: { order: 3, label: 'Labels', widget: 'label', emptyPolicy: 'empty-object' }
}

export const templateHints: Record<string, UiHint> = {
	name: { order: 1, placeholder: 'Name of template' },
	labels: { order: 2, label: 'Labels', widget: 'label', emptyPolicy: 'empty-object' }
}

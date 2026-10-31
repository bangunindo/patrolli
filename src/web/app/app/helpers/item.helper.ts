// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { ItemCategory } from '@@/shared/types/item.type'

export function getItemCategory(
	item:
		| { host_id?: string | null; template_item_id?: string | null; template_id?: string | null }
		| null
		| undefined,
	mode: string,
	isTemplateContext: boolean = false
): ItemCategory {
	if (mode === 'ADD') {
		return isTemplateContext ? ItemCategory.TEMPLATE_ROOT : ItemCategory.HOST_ONLY
	}

	const hasHost = Boolean(item?.host_id)
	const isInheritedOrApplied = Boolean(item?.template_item_id || item?.template_id)

	if (hasHost) {
		return isInheritedOrApplied ? ItemCategory.APPLIED_TEMPLATE : ItemCategory.HOST_ONLY
	} else {
		return isInheritedOrApplied ? ItemCategory.TEMPLATE_INHERITED : ItemCategory.TEMPLATE_ROOT
	}
}

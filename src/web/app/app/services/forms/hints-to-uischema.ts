// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type {
	ControlElement,
	GroupLayout,
	JsonSchema,
	UISchemaElement,
	VerticalLayout
} from '@jsonforms/core'
import type { UiHint } from '@@/shared/types/dynamic-form.type'

/**
 * Resolves UiHint for a given path key, checking exact match, alias, suffix match, and branch alias.
 */
export function resolveHintForPath(
	pathKey: string,
	hints: Record<string, UiHint> = {},
	rootData?: any
): UiHint | undefined {
	if (!pathKey) return undefined
	if (hints[pathKey]) return hints[pathKey]

	// Strip leading/trailing dots
	const cleanKey = pathKey.replace(/^\.+|\.+$/g, '')
	if (hints[cleanKey]) return hints[cleanKey]

	const parts = cleanKey.split('.')
	const leaf = parts[parts.length - 1]

	// Check with 'config.config' alias or active protocol/module type
	if (parts[0] === 'config') {
		const activeType =
			rootData?.config?.module || rootData?.config?.type || rootData?.module || rootData?.type
		if (activeType && parts.length > 2) {
			const branchAlias = ['config', activeType, ...parts.slice(2)].join('.')
			if (hints[branchAlias]) return hints[branchAlias]
			// Also check branchAlias with '.config.' inserted: e.g. config.dummy.config.payload...
			const branchWithConfig = ['config', activeType, 'config', ...parts.slice(2)].join('.')
			if (hints[branchWithConfig]) return hints[branchWithConfig]
		}
		if (parts[1] !== 'config') {
			const configAlias = ['config', 'config', ...parts.slice(1)].join('.')
			if (hints[configAlias]) return hints[configAlias]
		}
	}

	// Try matching suffixes in hints (e.g., '.auth', '.url')
	const dotLeaf = '.' + leaf
	for (const [k, h] of Object.entries(hints)) {
		if (k === leaf || k.endsWith(dotLeaf)) {
			// Ensure it doesn't match an incompatible protocol branch
			const activeType = rootData?.config?.type || rootData?.type
			const kParts = k.split('.')
			if (kParts[0] === 'config' && kParts.length > 2 && activeType) {
				const branch = kParts[1]
				if (branch !== 'config' && branch !== activeType) {
					continue
				}
			}
			return h
		}
	}

	return undefined
}

/**
 * Creates a ControlElement with options populated from resolved UiHint
 */
export function createControlElement(
	propKey: string,
	fullPath: string,
	hints: Record<string, UiHint> = {},
	rootData?: any
): ControlElement {
	const hint =
		resolveHintForPath(fullPath, hints, rootData) ||
		resolveHintForPath(propKey, hints, rootData) ||
		{}
	return {
		type: 'Control',
		scope: `#/properties/${propKey}`,
		...(hint.label ? { label: hint.label } : {}),
		options: {
			placeholder: hint.placeholder,
			description: hint.description,
			helpText: hint.helpText,
			sensitive: hint.sensitive,
			widget: hint.widget,
			options: hint.options,
			disabled: hint.disabled,
			readonly: hint.readonly,
			rules: hint.rules,
			format: hint.format,
			...hint
		}
	}
}

/**
 * Converts UiHint declarations and JsonSchema into a JSON Forms UISchema tree.
 * Groups properties into GroupLayout if hint defines 'group',
 * sorts according to 'order', and injects visual options.
 */
export function hintsToUiSchema(
	schema: JsonSchema,
	hints: Record<string, UiHint> = {}
): UISchemaElement {
	if (!schema || schema.type !== 'object' || !schema.properties) {
		// Fallback for non-object root or union
		return {
			type: 'VerticalLayout',
			elements: []
		} as VerticalLayout
	}

	const properties = schema.properties
	const entries = Object.keys(properties)
		.filter((propKey) => !hints[propKey]?.hidden)
		.map((propKey) => {
			const hint = hints[propKey] || {}
			const control: ControlElement = {
				type: 'Control',
				scope: `#/properties/${propKey}`,
				...(hint.label ? { label: hint.label } : {}),
				options: {
					placeholder: hint.placeholder,
					description: hint.description,
					helpText: hint.helpText,
					sensitive: hint.sensitive,
					widget: hint.widget,
					options: hint.options,
					disabled: hint.disabled,
					readonly: hint.readonly,
					rules: hint.rules,
					format: hint.format,
					...hint
				}
			}

			return {
				propKey,
				order: hint.order ?? 999,
				group: hint.group,
				control
			}
		})

	// Sort by order value
	entries.sort((a, b) => a.order - b.order)

	// Separate elements having 'group' from standalone elements
	const groupsMap = new Map<string, UISchemaElement[]>()
	const groupOptionsMap = new Map<string, Record<string, any>>()
	const ungroupedElements: UISchemaElement[] = []

	for (const entry of entries) {
		if (entry.group) {
			if (!groupsMap.has(entry.group)) {
				groupsMap.set(entry.group, [])
			}
			groupsMap.get(entry.group)!.push(entry.control)
			if (entry.control.options?.groupVariant) {
				groupOptionsMap.set(entry.group, { variant: entry.control.options.groupVariant })
			}
		} else {
			ungroupedElements.push(entry.control)
		}
	}

	const finalElements: UISchemaElement[] = [...ungroupedElements]

	for (const [groupTitle, groupElements] of groupsMap.entries()) {
		const groupLayout: GroupLayout = {
			type: 'Group',
			label: groupTitle,
			elements: groupElements,
			options: groupOptionsMap.get(groupTitle)
		}
		finalElements.push(groupLayout)
	}

	return {
		type: 'VerticalLayout',
		elements: finalElements
	} as VerticalLayout
}

// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { z } from 'zod'
import { flattenUnionOptions, unwrapZodType } from './zod-introspector'

/**
 * Checks if a value is a masked Sensitive secret object from the backend: { set: "<handle>" }
 */
export function isSensitiveSetObject(val: unknown): val is { set: string } {
	return (
		typeof val === 'object' &&
		val !== null &&
		'set' in val &&
		Object.keys(val).length === 1 &&
		typeof (val as { set: unknown }).set === 'string' &&
		(val as { set: string }).set.trim().length > 0
	)
}

/**
 * Checks if a Zod schema matches the Sensitive contract:
 * z.union([z.string(), z.object({ set: z.string() })])
 * Handles optional/nullable wrappers and nested unions (e.g. Orval nullable unions).
 */
export function isSensitiveSchema(schema: z.ZodType | undefined | null): boolean {
	if (!schema) return false

	const allOptions = flattenUnionOptions(schema)
	const nonNullOptions = allOptions.filter((opt) => {
		const u = unwrapZodType(opt)
		return u.typeName !== 'null' && u.typeName !== 'undefined'
	})

	if (nonNullOptions.length !== 2) return false

	const hasString = nonNullOptions.some((opt) => unwrapZodType(opt).typeName === 'string')
	const hasSetObject = nonNullOptions.some((opt) => {
		const u = unwrapZodType(opt)
		if (u.typeName !== 'object') return false
		const shape = (u.innerType as z.ZodObject<Record<string, z.ZodType>>).shape
		if (!shape) return false
		const keys = Object.keys(shape)
		return (
			keys.length === 1 && keys[0] === 'set' && unwrapZodType(shape.set).typeName === 'string'
		)
	})

	return hasString && hasSetObject
}

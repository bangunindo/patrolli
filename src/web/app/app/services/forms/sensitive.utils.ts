// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { z } from 'zod'
import { unwrapZodType } from './zod-introspector'

/**
 * Checks if a value is a masked Sensitive secret object from the backend: { set: "<handle>" }
 */
export function isSensitiveSetObject(val: unknown): val is { set: string } {
	return (
		typeof val === 'object' &&
		val !== null &&
		'set' in val &&
		typeof (val as { set: unknown }).set === 'string' &&
		(val as { set: string }).set.trim().length > 0
	)
}

/**
 * Recursively unwraps wrappers and flattens nested ZodUnion options.
 */
function collectZodUnionOptions(schema: z.ZodTypeAny): z.ZodTypeAny[] {
	const unwrapped = unwrapZodType(schema)
	if (unwrapped.typeName === 'ZodUnion') {
		const def = (unwrapped.innerType as z.ZodUnion<[z.ZodTypeAny, ...z.ZodTypeAny[]]>)._def
		const options: z.ZodTypeAny[] = []
		for (const opt of def.options) {
			options.push(...collectZodUnionOptions(opt))
		}
		return options
	}
	return [unwrapped.innerType]
}

/**
 * Checks if a Zod schema matches the Sensitive contract:
 * z.union([z.string(), z.object({ set: z.string() })])
 * Handles optional/nullable wrappers and nested unions (e.g. Orval nullable unions).
 */
export function isSensitiveSchema(schema: z.ZodTypeAny | undefined | null): boolean {
	if (!schema) return false

	const allOptions = collectZodUnionOptions(schema)
	// Filter out null / undefined branches from nullable unions
	const nonNullOptions = allOptions.filter((opt) => {
		const u = unwrapZodType(opt)
		return u.typeName !== 'ZodNull' && u.typeName !== 'ZodUndefined'
	})

	if (nonNullOptions.length !== 2) return false

	const hasString = nonNullOptions.some((opt) => unwrapZodType(opt).typeName === 'ZodString')
	const hasSetObject = nonNullOptions.some((opt) => {
		const u = unwrapZodType(opt)
		if (u.typeName !== 'ZodObject') return false
		const shape = (u.innerType as z.ZodObject<Record<string, z.ZodTypeAny>>).shape
		if (!shape) return false
		const keys = Object.keys(shape)
		return (
			keys.length === 1 &&
			keys[0] === 'set' &&
			unwrapZodType(shape.set).typeName === 'ZodString'
		)
	})

	return hasString && hasSetObject
}

/**
 * Recursively flattens anyOf / oneOf branches from a JSON Schema.
 */
function collectJsonSchemaBranches(schema: any): any[] {
	if (!schema || typeof schema !== 'object') return []
	const branches = schema.anyOf || schema.oneOf
	if (Array.isArray(branches)) {
		const list: any[] = []
		for (const branch of branches) {
			list.push(...collectJsonSchemaBranches(branch))
		}
		return list
	}
	return [schema]
}

const nullableJsonSchemaCache = new WeakMap<object, boolean>()
const sensitiveJsonSchemaCache = new WeakMap<object, boolean>()

/**
 * Checks if a Draft-07 JSON Schema allows null (nullable field).
 */
export function isNullableJsonSchema(schema: any): boolean {
	if (!schema || typeof schema !== 'object') return false
	const cached = nullableJsonSchemaCache.get(schema)
	if (cached !== undefined) return cached

	let result = false
	if (schema.type === 'null') {
		result = true
	} else {
		const branches = schema.anyOf || schema.oneOf
		if (Array.isArray(branches)) {
			result = branches.some((b: any) => isNullableJsonSchema(b))
		}
	}

	nullableJsonSchemaCache.set(schema, result)
	return result
}

/**
 * Checks if a Draft-07 JSON Schema corresponds to a Sensitive union.
 * Handles nested anyOf / oneOf trees and nullable branches.
 */
export function isSensitiveJsonSchema(schema: any): boolean {
	if (!schema || typeof schema !== 'object') return false
	const cached = sensitiveJsonSchemaCache.get(schema)
	if (cached !== undefined) return cached

	const allBranches = collectJsonSchemaBranches(schema)
	if (allBranches.length < 2) {
		sensitiveJsonSchemaCache.set(schema, false)
		return false
	}

	// Filter out null branches
	const nonNullBranches = allBranches.filter((b: any) => b && b.type !== 'null')
	if (nonNullBranches.length !== 2) {
		sensitiveJsonSchemaCache.set(schema, false)
		return false
	}

	const hasString = nonNullBranches.some((b: any) => b.type === 'string')
	const hasSetObject = nonNullBranches.some(
		(b: any) =>
			b.type === 'object' &&
			b.properties &&
			b.properties.set &&
			(b.properties.set.type === 'string' || b.properties.set.type === undefined)
	)

	const result = hasString && hasSetObject
	sensitiveJsonSchemaCache.set(schema, result)
	return result
}

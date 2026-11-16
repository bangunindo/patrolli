// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { z } from 'zod'

/**
 * Recursively merges schemas inside allOf arrays into unified object properties & required lists.
 * Required because Zod Intersection (.and(...)) produces allOf structures in JSON Schema
 * that are not automatically expanded by standard property-based validators/renderers.
 */
export function mergeAllOf(schema: any): any {
	if (!schema || typeof schema !== 'object') return schema
	if (Array.isArray(schema)) return schema.map(mergeAllOf)

	const res = { ...schema }

	if (Array.isArray(res.allOf)) {
		const mergedProps: Record<string, any> = {}
		const mergedRequired = new Set<string>(res.required || [])

		for (const sub of res.allOf) {
			const resolvedSub = mergeAllOf(sub)
			if (resolvedSub && resolvedSub.properties) {
				Object.assign(mergedProps, resolvedSub.properties)
			}
			if (Array.isArray(resolvedSub?.required)) {
				resolvedSub.required.forEach((r: string) => mergedRequired.add(r))
			}
		}

		delete res.allOf
		res.type = res.type || 'object'
		res.properties = { ...mergedProps, ...(res.properties || {}) }
		if (mergedRequired.size > 0) {
			res.required = Array.from(mergedRequired)
		}
	}

	for (const k of Object.keys(res)) {
		if (k !== 'properties' && typeof res[k] === 'object') {
			res[k] = mergeAllOf(res[k])
		}
	}

	if (res.properties) {
		for (const pk of Object.keys(res.properties)) {
			res.properties[pk] = mergeAllOf(res.properties[pk])
		}
	}

	return res
}

/**
 * Converts a Zod schema to a Draft-07 JSON Schema with merged allOf object properties.
 */
export function convertZodToJsonSchema(schema: z.ZodType): Record<string, any> {
	const rawSchema = z.toJSONSchema(schema, {
		target: 'draft-07',
		unrepresentable: 'any'
	}) as any

	if (rawSchema.$schema) {
		delete rawSchema.$schema
	}

	return mergeAllOf(rawSchema) as Record<string, any>
}

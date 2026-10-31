// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { zodToJsonSchema } from 'zod-to-json-schema'
import type { z } from 'zod'
import type { JsonSchema } from '@jsonforms/core'

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

	// Normalize nullable primitive unions (e.g. anyOf: [{type: 'integer'}, {type: 'null'}])
	// to type: ['integer', 'null'] so JSON Forms uses primitive control renderers instead of UnionControl
	if (Array.isArray(res.anyOf) && res.anyOf.length === 2) {
		const nullIdx = res.anyOf.findIndex((b: any) => b && b.type === 'null')
		if (nullIdx !== -1) {
			const other = res.anyOf[1 - nullIdx]
			if (
				other &&
				typeof other.type === 'string' &&
				['string', 'number', 'integer', 'boolean'].includes(other.type) &&
				!other.anyOf &&
				!other.oneOf &&
				!other.allOf &&
				!other.properties
			) {
				delete res.anyOf
				Object.assign(res, other)
				res.type = [other.type, 'null']
			}
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
 * Converts Zod Schema to Draft-07 JSON Schema compatible with JSON Forms.
 * Inlines $ref references so the schema is self-contained and easily consumed by JSON Forms parsers.
 */
export function convertZodToJsonSchema(schema: z.ZodTypeAny): JsonSchema {
	const rawSchema = zodToJsonSchema(schema, {
		target: 'jsonSchema7',
		$refStrategy: 'none'
	}) as any

	// Remove unnecessary top-level properties not required by JSON Forms if present
	if (rawSchema.$schema) {
		delete rawSchema.$schema
	}

	// Normalize allOf into flat object properties
	return mergeAllOf(rawSchema) as JsonSchema
}

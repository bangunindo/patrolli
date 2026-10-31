// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { formatZodSchemaForMonaco, isZodSchema } from '@/app/helpers/monaco.helper'
import { Dummy } from '@@/shared/generated/patrolliAPI.schemas.type'
import { describe, expect, it } from 'vitest'
import { z } from 'zod'

describe('monaco.helper', () => {
	describe('isZodSchema', () => {
		it('should return true for zod objects', () => {
			expect(isZodSchema(z.object({ a: z.string() }))).toBe(true)
			expect(isZodSchema(Dummy)).toBe(true)
		})

		it('should return false for plain objects or primitives', () => {
			expect(isZodSchema({})).toBe(false)
			expect(isZodSchema({ type: 'object' })).toBe(false)
			expect(isZodSchema(null)).toBe(false)
			expect(isZodSchema(undefined)).toBe(false)
		})
	})

	describe('formatZodSchemaForMonaco', () => {
		it('should return empty object for null or undefined', () => {
			expect(formatZodSchemaForMonaco(null)).toEqual({})
			expect(formatZodSchemaForMonaco(undefined)).toEqual({})
		})

		it('should convert zod schema into valid draft-07 json schema', () => {
			const schema = z.object({
				name: z.string(),
				count: z.number().int().optional()
			})

			const result = formatZodSchemaForMonaco(schema)

			expect(result.$schema).toBe('http://json-schema.org/draft-07/schema#')
			expect(result.type).toBe('object')
			expect(result.properties).toBeDefined()
			expect(result.properties.name.type).toBe('string')
			expect(result.properties.count.type).toBe('integer')
			expect(result.required).toContain('name')
			expect(result.required).not.toContain('count')
		})

		it('should convert patrolli API Dummy zod schema properly', () => {
			const result = formatZodSchemaForMonaco(Dummy)

			expect(result.$schema).toBe('http://json-schema.org/draft-07/schema#')
			expect(result.type).toBe('object')
			expect(result.properties).toBeDefined()
			expect(result.properties.count).toBeDefined()
			expect(result.properties.payload).toBeDefined()
		})

		it('should preserve and normalize raw json schema', () => {
			const rawJsonSchema = {
				type: 'object',
				properties: {
					foo: { type: 'string' }
				}
			}

			const result = formatZodSchemaForMonaco(rawJsonSchema)

			expect(result.$schema).toBe('http://json-schema.org/draft-07/schema#')
			expect(result.type).toBe('object')
			expect(result.properties.foo.type).toBe('string')
		})
	})
})

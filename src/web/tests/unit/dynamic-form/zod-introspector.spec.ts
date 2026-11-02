// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from 'vitest'
import { z } from 'zod'
import { introspectZodSchema, unwrapZodType } from '@/app/services/forms/zod-introspector'

describe('zod-introspector', () => {
	describe('unwrapZodType', () => {
		it('should properly unwrap ZodOptional, ZodNullable, and ZodDefault wrappers', () => {
			const schema = z.string().min(3).optional().nullable().default('test')
			const unwrapped = unwrapZodType(schema)

			expect(unwrapped.typeName).toBe('ZodString')
			expect(unwrapped.isOptional).toBe(true)
			expect(unwrapped.isNullable).toBe(true)
			expect(unwrapped.hasDefault).toBe(true)
			expect(unwrapped.defaultValue).toBe('test')
		})

		it('should extract OpenAPI description', () => {
			const schema = z.string().describe('Host target IP or FQDN')
			const unwrapped = unwrapZodType(schema)

			expect(unwrapped.description).toBe('Host target IP or FQDN')
		})
	})

	describe('introspectZodSchema - Basic Types', () => {
		it('should infer standard field types with validation constraints', () => {
			const schema = z.object({
				name: z.string().min(1).max(50),
				retries: z.number().int().min(0).max(10).default(3),
				is_active: z.boolean().default(true),
				protocol: z.enum(['udp', 'tcp']),
				tags: z.array(z.string()).optional()
			})

			const descriptors = introspectZodSchema(schema)
			expect(descriptors.length).toBe(5)

			const nameDesc = descriptors.find((d) => d.name === 'name')!
			expect(nameDesc.widget).toBe('text')
			expect(nameDesc.required).toBe(true)
			expect(nameDesc.min).toBe(1)
			expect(nameDesc.max).toBe(50)

			const retriesDesc = descriptors.find((d) => d.name === 'retries')!
			expect(retriesDesc.widget).toBe('number')
			expect(retriesDesc.step).toBe(1)
			expect(retriesDesc.min).toBe(0)
			expect(retriesDesc.max).toBe(10)
			expect(retriesDesc.defaultValue).toBe(3)

			const activeDesc = descriptors.find((d) => d.name === 'is_active')!
			expect(activeDesc.widget).toBe('switch')

			const protoDesc = descriptors.find((d) => d.name === 'protocol')!
			expect(protoDesc.widget).toBe('select')
			expect(protoDesc.options).toEqual([
				{ label: 'Udp', value: 'udp' },
				{ label: 'Tcp', value: 'tcp' }
			])

			const tagsDesc = descriptors.find((d) => d.name === 'tags')!
			expect(tagsDesc.widget).toBe('array-string')
			expect(tagsDesc.isOptional).toBe(true)
		})
	})

	describe('introspectZodSchema - Discriminated Unions', () => {
		it('should detect discriminated union based on type property', () => {
			const snmpV1 = z.object({
				type: z.literal('snmp_v1'),
				community: z.string()
			})
			const http = z.object({
				type: z.literal('http'),
				url: z.string()
			})
			const schema = z.object({
				config: z.discriminatedUnion('type', [snmpV1, http])
			})

			const descriptors = introspectZodSchema(schema)
			const configDesc = descriptors.find((d) => d.name === 'config')!

			expect(configDesc.widget).toBe('discriminated-union')
			expect(configDesc.unionDiscriminator).toBe('type')
			expect(configDesc.options).toEqual([
				{ label: 'SNMP V1', value: 'snmp_v1' },
				{ label: 'HTTP', value: 'http' }
			])
			expect(configDesc.unionBranches?.snmp_v1).toBeDefined()
			expect(configDesc.unionBranches?.http).toBeDefined()
		})
	})

	describe('introspectZodSchema - UI Hints Integration', () => {
		it('should apply overrides from UI hints (sensitive, label, order)', () => {
			const schema = z.object({
				username: z.string(),
				password: z.string(),
				priority: z.number()
			})

			const hints = {
				password: { sensitive: true, label: 'Secret Credential', order: 1 },
				username: { order: 2 }
			}

			const descriptors = introspectZodSchema(schema, [], hints)

			// Sorted by order
			expect(descriptors[0].name).toBe('password')
			expect(descriptors[0].widget).toBe('password')
			expect(descriptors[0].label).toBe('Secret Credential')

			expect(descriptors[1].name).toBe('username')
			expect(descriptors[2].name).toBe('priority')
		})
	})
})

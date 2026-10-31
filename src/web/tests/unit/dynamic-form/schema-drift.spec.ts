// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from 'vitest'
import {
	CreateEndpointRequest,
	CreateHostItemRequest,
	CreateHostRequest,
	CreateListenerRequest,
	CreateProcessorRequest,
	CreateSinkRequest,
	CreateTemplateItemRequest
} from '@@/shared/generated/patrolliAPI.schemas.type'
import { introspectZodSchema } from '@/app/services/forms/zod-introspector'
import { AutoFormAdapter } from '@/app/services/forms/auto-adapter'
import { endpointHints } from '@/app/services/forms/ui-hints/endpoint.hints'
import { listenerHints } from '@/app/services/forms/ui-hints/listener.hints'
import { sinkHints } from '@/app/services/forms/ui-hints/sink.hints'
import { hostHints } from '@/app/services/forms/ui-hints/host.hints'
import { processorHints } from '@/app/services/forms/ui-hints/processor.hints'
import { itemHints } from '@/app/services/forms/ui-hints/item.hints'

import { z } from 'zod'

describe('Automated CI Schema Drift Verification', () => {
	const entities: Array<{
		name: string
		schema: z.ZodTypeAny
		hints: Record<string, any>
	}> = [
		{ name: 'Endpoint', schema: CreateEndpointRequest, hints: endpointHints },
		{ name: 'Host', schema: CreateHostRequest, hints: hostHints },
		{ name: 'Listener', schema: CreateListenerRequest, hints: listenerHints },
		{ name: 'Processor', schema: CreateProcessorRequest, hints: processorHints },
		{ name: 'Sink', schema: CreateSinkRequest, hints: sinkHints },
		{ name: 'Host Item', schema: CreateHostItemRequest, hints: itemHints },
		{ name: 'Template Item', schema: CreateTemplateItemRequest, hints: itemHints }
	]

	for (const entity of entities) {
		describe(`Entity: ${entity.name}`, () => {
			it('dapat diintrospeksi tanpa error dan menghasilkan FieldDescriptor valid', () => {
				const descriptors = introspectZodSchema(entity.schema, [], entity.hints)
				expect(descriptors.length).toBeGreaterThan(0)

				for (const desc of descriptors) {
					expect(desc.name).toBeDefined()
					expect(desc.path.length).toBeGreaterThan(0)
					expect(desc.widget).toBeDefined()
					// Ensure no field inadvertently falls back to the generic handler
					expect(desc.widget).not.toBe('fallback')
				}
			})

			it('memvalidasi payload draft secara terkontrol via AutoFormAdapter', () => {
				const draft: Record<string, any> = {}
				const result = AutoFormAdapter.toRequest(entity.schema, draft, entity.hints)

				// Patrolli create schema has required fields (like name), so empty draft must produce issues
				expect(result.success).toBe(false)
				if (!result.success) {
					expect(Object.keys(result.issues).length).toBeGreaterThan(0)
					// At least name is required
					expect(result.issues['name']).toBeDefined()
				}
			})
		})
	}
})

// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { convertZodToJsonSchema } from '@/app/services/forms/json-schema-converter'
import { z } from 'zod'

export function isZodSchema(input: any): boolean {
	return (
		input instanceof z.ZodType ||
		(Boolean(input) && typeof input === 'object' && '_def' in input)
	)
}

export function formatZodSchemaForMonaco(rawInput: any): any {
	if (!rawInput) return {}

	let schema: any
	if (isZodSchema(rawInput)) {
		schema = convertZodToJsonSchema(rawInput)
	} else {
		schema = JSON.parse(JSON.stringify(rawInput))
	}

	if (!schema.$schema) {
		schema.$schema = 'http://json-schema.org/draft-07/schema#'
	}

	if (schema.$ref && schema.$ref.startsWith('#/')) {
		const pathParts = schema.$ref.split('/').slice(1)
		let targetDefinition = schema

		for (const part of pathParts) {
			if (targetDefinition && targetDefinition[part] !== undefined) {
				targetDefinition = targetDefinition[part]
			} else {
				targetDefinition = null
				break
			}
		}

		if (targetDefinition && typeof targetDefinition === 'object') {
			const savedDefs = schema.$defs || schema.definitions
			const savedSchemaUri = schema.$schema

			schema = {
				$schema: savedSchemaUri,
				...targetDefinition
			}

			if (savedDefs) {
				schema.$defs = savedDefs
			}

			delete schema.$ref
		}
	}

	return schema
}

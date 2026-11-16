// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { toRaw } from 'vue'
import { z } from 'zod'
import type { FieldDescriptor, FieldPath, UiHint } from '@@/shared/types/dynamic-form.type'
import { formatFieldLabel, pathToString } from './path-helper'
import { isSensitiveSchema } from './sensitive.utils'

export interface UnwrappedZodType {
	innerType: z.ZodType
	typeName: string
	isOptional: boolean
	isNullable: boolean
	hasDefault: boolean
	defaultValue?: any
	description?: string
}

/**
 * Safely reads the internal definition object from a Zod v4 schema/check.
 */
export function getZodDef(schema: any): any {
	const raw = toRaw(schema)
	return raw?._zod?.def ?? raw?.def
}

/**
 * Recursively unwraps Zod wrappers (Optional, Nullable, Default, Pipe/Transform)
 * to extract the inner schema and constraint metadata.
 */
export function unwrapZodType(schema: z.ZodType | undefined | null): UnwrappedZodType {
	if (!schema) {
		return {
			innerType: undefined as any,
			typeName: 'unknown',
			isOptional: true,
			isNullable: true,
			hasDefault: false,
			defaultValue: undefined,
			description: undefined
		}
	}

	let current: z.ZodType = toRaw(schema)
	let isOptional = false
	let isNullable = false
	let hasDefault = false
	let defaultValue: any = undefined
	let description: string | undefined = current.description

	while (current) {
		current = toRaw(current)
		const def = getZodDef(current)
		const typeName = def?.type
		description = current.description || description

		if (typeName === 'optional') {
			isOptional = true
			current = def.innerType
		} else if (typeName === 'nullable') {
			isNullable = true
			current = def.innerType
		} else if (typeName === 'default' || typeName === 'prefault') {
			hasDefault = true
			try {
				const dv = def.defaultValue
				defaultValue = typeof dv === 'function' ? dv() : dv
			} catch {
				defaultValue = undefined
			}
			current = def.innerType
		} else if (typeName === 'catch' || typeName === 'readonly') {
			current = def.innerType
		} else if (typeName === 'pipe') {
			current = def.in
		} else if (typeName === 'lazy') {
			return {
				innerType: current,
				typeName: 'lazy',
				isOptional: true,
				isNullable: true,
				hasDefault: false,
				defaultValue: undefined,
				description
			}
		} else {
			break
		}
	}

	return {
		innerType: current,
		typeName: getZodDef(current)?.type || 'unknown',
		isOptional,
		isNullable,
		hasDefault,
		defaultValue,
		description
	}
}

/**
 * Extracts the literal value from a ZodLiteral or ZodEnum
 */
export function extractLiteralValue(schema: z.ZodType): string | undefined {
	const unwrapped = unwrapZodType(schema)
	const def = getZodDef(unwrapped.innerType)
	if (unwrapped.typeName === 'literal') {
		return def?.values?.[0] ?? def?.value
	}
	if (unwrapped.typeName === 'enum') {
		const opts = (unwrapped.innerType as any).options ?? Object.values(def?.entries || {})
		return opts[0]
	}
	return undefined
}

/**
 * Extracts object shape from ZodObject or ZodIntersection
 */
export function extractObjectShape(schema: z.ZodType): Record<string, z.ZodType> {
	const unwrapped = unwrapZodType(schema)
	if (unwrapped.typeName === 'object') {
		const obj = unwrapped.innerType as z.ZodObject<any>
		const def = getZodDef(obj)
		return obj.shape ?? (typeof def?.shape === 'function' ? def.shape() : def?.shape) ?? {}
	}
	if (unwrapped.typeName === 'intersection') {
		const def = getZodDef(unwrapped.innerType)
		const leftShape = extractObjectShape(def.left)
		const rightShape = extractObjectShape(def.right)
		return { ...leftShape, ...rightShape }
	}
	return {}
}

/**
 * Checks if a Zod schema is an AttributeFilter ({ and: [...] } | { or: [...] } | { match: ... })
 */
export function isAttributeFilterSchema(schema: z.ZodType): boolean {
	const unwrapped = unwrapZodType(schema)
	if (unwrapped.typeName !== 'union') return false
	const options = getZodDef(unwrapped.innerType)?.options || []
	if (options.length < 3) return false

	let hasAnd = false
	let hasOr = false
	let hasMatch = false

	for (const opt of options) {
		const optUnwrapped = unwrapZodType(opt)
		if (optUnwrapped.typeName === 'object') {
			const shape = extractObjectShape(optUnwrapped.innerType)
			if ('and' in shape) hasAnd = true
			if ('or' in shape) hasOr = true
			if ('match' in shape) hasMatch = true
		}
	}

	return hasAnd && hasOr && hasMatch
}

export function flattenUnionOptions(schema: z.ZodType): z.ZodType[] {
	const unwrapped = unwrapZodType(schema)
	const rawOptions =
		getZodDef(unwrapped.innerType)?.options || (unwrapped.innerType as any).options || []
	const result: z.ZodType[] = []
	for (const opt of rawOptions) {
		const optUnwrapped = unwrapZodType(opt)
		if (optUnwrapped.typeName === 'union') {
			result.push(...flattenUnionOptions(optUnwrapped.innerType))
		} else {
			result.push(opt)
		}
	}
	return result
}

function sortSelectOptions(
	selectOptions: Array<{ label: string; value: string }>,
	parentHint: UiHint
) {
	if (parentHint.optionsOrder && parentHint.optionsOrder.length > 0) {
		selectOptions.sort((a, b) => {
			const idxA = parentHint.optionsOrder!.indexOf(a.value)
			const idxB = parentHint.optionsOrder!.indexOf(b.value)
			return (idxA >= 0 ? idxA : 999) - (idxB >= 0 ? idxB : 999)
		})
	} else if (parentHint.discriminatorDefault) {
		const defVal = parentHint.discriminatorDefault
		selectOptions.sort((a, b) => {
			if (a.value === defVal) return -1
			if (b.value === defVal) return 1
			return 0
		})
	}
}

/**
 * Resolves discriminated unions (both internal and external tagged)
 * into discriminator options and branch descriptors.
 */
function resolveUnion(
	unionSchema: z.ZodType,
	basePath: FieldPath,
	hints: Record<string, UiHint>,
	isNullableUnion = false
): {
	discriminator: string
	tagMode: 'internal' | 'external'
	options: Array<{ label: string; value: string }>
	branches: Record<string, FieldDescriptor[]>
} {
	const rawOptions = flattenUnionOptions(unionSchema)
	const parentHint = hints[pathToString(basePath)] || {}
	const nonNullOptions = rawOptions.filter(
		(opt: z.ZodType) =>
			unwrapZodType(opt).typeName !== 'null' && unwrapZodType(opt).typeName !== 'undefined'
	)
	const hasNull = rawOptions.length > nonNullOptions.length || isNullableUnion

	// 1. Check if this is an internally tagged discriminated union (has discriminator fields like 'type', 'module', 'kind', 'method', 'strategy')
	const candidateDiscriminators = [
		parentHint.discriminatorField,
		'type',
		'module',
		'kind',
		'method',
		'strategy'
	].filter(Boolean) as string[]

	let foundDiscKey: string | undefined
	for (const cand of candidateDiscriminators) {
		const allMatch =
			nonNullOptions.length > 0 &&
			nonNullOptions.every((opt: z.ZodType) => {
				const unwrapped = unwrapZodType(opt)
				if (unwrapped.typeName === 'object' || unwrapped.typeName === 'intersection') {
					const shape = extractObjectShape(unwrapped.innerType)
					return cand in shape && extractLiteralValue(shape[cand]) !== undefined
				}
				return false
			})
		if (allMatch) {
			foundDiscKey = cand
			break
		}
	}

	if (foundDiscKey) {
		const discriminator = foundDiscKey
		const branches: Record<string, FieldDescriptor[]> = {}
		const selectOptions: Array<{ label: string; value: string }> = []

		for (const optSchema of nonNullOptions) {
			const unwrapped = unwrapZodType(optSchema)
			if (unwrapped.typeName === 'object' || unwrapped.typeName === 'intersection') {
				const shape = extractObjectShape(unwrapped.innerType)
				const discVal = extractLiteralValue(shape[discriminator])

				if (discVal !== undefined) {
					selectOptions.push({
						label: formatFieldLabel(discVal),
						value: discVal
					})

					// Build descriptors for this branch from the merged shape
					const branchDescriptors: FieldDescriptor[] = []
					// Support branch-specific UI hints (including all nested child levels)
					const branchPrefix = [...basePath, discVal].join('.') + '.'
					const directPrefix = basePath.length > 0 ? basePath.join('.') + '.' : ''
					const branchHints: Record<string, UiHint> = { ...hints }
					for (const [k, v] of Object.entries(hints)) {
						if (k.startsWith(branchPrefix)) {
							const remainder = k.slice(branchPrefix.length)
							const mappedKey = directPrefix + remainder
							branchHints[mappedKey] = {
								...(branchHints[mappedKey] || {}),
								...v
							}
							if ('config' in shape) {
								if (!remainder.startsWith('config.')) {
									const unnestedKey = directPrefix + 'config.' + remainder
									branchHints[unnestedKey] = {
										...(branchHints[unnestedKey] || {}),
										...v
									}
								}
							}
						}
					}

					for (const [propKey, propSchema] of Object.entries(shape)) {
						if (propKey === discriminator) continue

						const desc = introspectField(propKey, propSchema, basePath, branchHints)
						if (propKey === 'config' && desc.hints?.hideGroup === undefined) {
							desc.hints = { ...(desc.hints || {}), hideGroup: true }
						}
						branchDescriptors.push(desc)
					}
					branchDescriptors.sort(
						(a, b) => (a.hints?.order ?? 9999) - (b.hints?.order ?? 9999)
					)
					branches[discVal] = branchDescriptors
				}
			}
		}

		if (hasNull || parentHint.optionsOrder?.includes('none')) {
			if (!selectOptions.some((o) => o.value === 'none')) {
				selectOptions.unshift({
					label: 'None',
					value: 'none'
				})
				branches['none'] = []
			}
		}

		sortSelectOptions(selectOptions, parentHint)

		return {
			discriminator,
			tagMode: 'internal',
			options: selectOptions,
			branches
		}
	}

	// 2. Nullable single-object union (e.g. z.union([z.null(), z.object(...)]) for optional sections like auth/privacy/tls/v2c)
	if (hasNull && nonNullOptions.length === 1) {
		const singleUnwrapped = unwrapZodType(nonNullOptions[0])
		if (singleUnwrapped.typeName === 'object' || singleUnwrapped.typeName === 'intersection') {
			const objShape = extractObjectShape(singleUnwrapped.innerType)
			const objFields = Object.entries(objShape).map(([k, v]) =>
				introspectField(k, v, basePath, hints)
			)
			objFields.sort((a, b) => (a.hints?.order ?? 9999) - (b.hints?.order ?? 9999))
			return {
				discriminator: '__enabled__',
				tagMode: 'internal',
				options: [
					{ label: 'None', value: 'none' },
					{
						label: formatFieldLabel(basePath[basePath.length - 1] || 'Enabled'),
						value: 'enabled'
					}
				],
				branches: {
					none: [],
					enabled: objFields
				}
			}
		}
	}

	// 3. Check if this is an externally-tagged union:
	// Each branch is a ZodObject with exactly 1 property (variant name)
	const isExternalTagged =
		nonNullOptions.length > 0 &&
		nonNullOptions.every((opt: z.ZodType) => {
			const unwrapped = unwrapZodType(opt)
			if (unwrapped.typeName === 'object' || unwrapped.typeName === 'intersection') {
				const shape = extractObjectShape(unwrapped.innerType)
				return Object.keys(shape).length === 1
			}
			return false
		})

	if (isExternalTagged) {
		const discriminator = parentHint.discriminatorField || '__variant__'
		const branches: Record<string, FieldDescriptor[]> = {}
		const selectOptions: Array<{ label: string; value: string }> = []

		if (hasNull || parentHint.optionsOrder?.includes('none')) {
			selectOptions.push({
				label: 'None',
				value: 'none'
			})
			branches['none'] = []
		}

		for (const optSchema of nonNullOptions) {
			const unwrapped = unwrapZodType(optSchema)
			const shape = extractObjectShape(unwrapped.innerType)
			const [variantKey] = Object.keys(shape)
			const payloadSchema = shape[variantKey]

			selectOptions.push({
				label: formatFieldLabel(variantKey),
				value: variantKey
			})

			const branchPrefix = [...basePath, variantKey].join('.') + '.'
			const directPrefix = basePath.length > 0 ? basePath.join('.') + '.' : ''
			const branchHints: Record<string, UiHint> = { ...hints }
			for (const [k, v] of Object.entries(hints)) {
				if (k.startsWith(branchPrefix)) {
					const remainder = k.slice(branchPrefix.length)
					const mappedKey = directPrefix + remainder
					branchHints[mappedKey] = {
						...(branchHints[mappedKey] || {}),
						...v
					}
				}
			}

			const payloadDesc = introspectField(variantKey, payloadSchema, basePath, branchHints)
			if (payloadDesc.widget === 'custom' && payloadDesc.hints?.hideGroup === undefined) {
				payloadDesc.hints = { ...(payloadDesc.hints || {}), hideGroup: true }
			}
			branches[variantKey] = [payloadDesc]
		}

		sortSelectOptions(selectOptions, parentHint)

		return {
			discriminator,
			tagMode: 'external',
			options: selectOptions,
			branches
		}
	}

	return {
		discriminator: 'type',
		tagMode: 'internal',
		options: [],
		branches: {}
	}
}

/**
 * Introspects a Zod property/field into a FieldDescriptor
 */
export function introspectField(
	key: string,
	rawSchema: z.ZodType,
	basePath: FieldPath = [],
	hints: Record<string, UiHint> = {}
): FieldDescriptor {
	const path = [...basePath, key]
	const pathKey = pathToString(path)
	let hint = hints[pathKey]
	if (!hint) {
		const dotKey = '.' + key
		for (const [hk, hv] of Object.entries(hints)) {
			// Prevent cross-branch contamination (e.g. config.snmp_trap.* matching inside syslog)
			const hkParts = hk.split('.')
			if (hkParts.length > 2 && hkParts[0] === 'config') {
				const branchName = hkParts[1]
				if (branchName !== 'config' && !path.includes(branchName)) {
					continue
				}
			}
			if (hk === key || hk.endsWith(dotKey)) {
				hint = hv
				break
			}
		}
	}
	hint = hint || {}

	const unwrapped = unwrapZodType(rawSchema)
	const inner = unwrapped.innerType
	const typeName = unwrapped.typeName

	let widget: string = 'text'
	let sensitive: boolean = hint.sensitive || false
	let defaultValue: any = undefined
	let min: number | undefined
	let max: number | undefined
	let step: number | undefined
	let format: string | undefined
	const rules: string[] = []
	let pattern: string | undefined
	let regex: RegExp | undefined
	let options: Array<{ label: string; value: any }> | undefined
	let children: FieldDescriptor[] | undefined
	let unionDiscriminator: string | undefined
	let unionTagMode: 'internal' | 'external' | undefined
	let unionBranches: Record<string, FieldDescriptor[]> | undefined
	let isNullable = unwrapped.isNullable
	const isOptional = unwrapped.isOptional

	const innerDef = getZodDef(inner)

	switch (typeName) {
		case 'string': {
			widget = hint.sensitive ? 'password' : 'text'
			const applyStringFormat = (fmt?: string, pat?: RegExp) => {
				if (fmt === 'uuid') {
					format = 'uuid'
					if (!rules.includes('uuid')) rules.push('uuid')
				} else if (fmt === 'email') {
					format = 'email'
					if (!rules.includes('email')) rules.push('email')
				} else if (fmt === 'ip' || fmt === 'ipv4' || fmt === 'ipv6') {
					format = 'ip'
					if (!rules.includes('ip')) rules.push('ip')
				} else if (fmt === 'url') {
					format = 'url'
					if (!rules.includes('url')) rules.push('url')
				} else if (fmt === 'regex' && pat) {
					regex = pat
					pattern = pat.source
				}
			}

			if (innerDef?.format) {
				applyStringFormat(innerDef.format, innerDef.pattern)
			}

			const checks = innerDef?.checks || []
			for (const check of checks) {
				const cDef = getZodDef(check)
				if (cDef?.check === 'min_length') min = cDef.minimum
				if (cDef?.check === 'max_length') max = cDef.maximum
				if (cDef?.check === 'string_format') {
					applyStringFormat(cDef.format, cDef.pattern)
				}
			}
			break
		}

		case 'number':
		case 'int': {
			widget = 'number'
			if (typeName === 'int' || innerDef?.format === 'safeint') {
				step = 1
			}
			const checks = innerDef?.checks || []
			for (const check of checks) {
				const cDef = getZodDef(check)
				if (cDef?.check === 'greater_than') min = cDef.value
				if (cDef?.check === 'less_than') max = cDef.value
				if (cDef?.check === 'number_format' && cDef.format === 'safeint') step = 1
			}
			break
		}

		case 'boolean': {
			widget = 'switch'
			break
		}

		case 'enum': {
			widget = 'select'
			const entries = innerDef?.entries || {}
			const values: any[] = Array.isArray((inner as any).options)
				? (inner as any).options
				: Object.values(entries)
			options = values.map((val: any) => ({
				label: formatFieldLabel(String(val)),
				value: val
			}))
			break
		}

		case 'array': {
			defaultValue = []
			const elementSchema = innerDef?.element ?? innerDef?.type
			const elemUnwrapped = unwrapZodType(elementSchema)
			if (elemUnwrapped.typeName === 'string') {
				widget = 'array-string'
			} else if (elemUnwrapped.typeName === 'object') {
				widget = 'array-object'
				children = introspectObjectSchema(
					elemUnwrapped.innerType as z.ZodObject<any>,
					path,
					hints
				)
			} else if (
				elemUnwrapped.typeName === 'tuple' &&
				(getZodDef(elemUnwrapped.innerType)?.items?.length ?? 0) === 2
			) {
				widget = 'log-attrs'
			} else {
				widget = 'array-string'
			}
			break
		}

		case 'record': {
			defaultValue = {}
			widget = 'record'
			const rec = inner as z.ZodRecord<any, any>
			const keySchema = innerDef?.keyType || (rec as any).keySchema
			const valueSchema = innerDef?.valueType || (rec as any).valueSchema
			if (keySchema) {
				const keyUnwrapped = unwrapZodType(keySchema)
				if (keyUnwrapped.typeName === 'enum') {
					const keyDef = getZodDef(keyUnwrapped.innerType)
					const values: any[] = Array.isArray((keyUnwrapped.innerType as any).options)
						? (keyUnwrapped.innerType as any).options
						: Object.values(keyDef?.entries || {})
					options = values.map((val: any) => ({
						label: formatFieldLabel(String(val)),
						value: val
					}))
				}
			}
			if (valueSchema) {
				children = [introspectField('value', valueSchema, path, hints)]
			}
			break
		}

		case 'unknown':
		case 'any': {
			widget = 'json'
			break
		}

		case 'object': {
			widget = 'custom'
			children = introspectObjectSchema(inner as z.ZodObject<any>, path, hints)
			break
		}

		case 'intersection': {
			widget = 'custom'
			const shape = extractObjectShape(inner)
			children = []
			for (const [propKey, propSchema] of Object.entries(shape)) {
				children.push(introspectField(propKey, propSchema, path, hints))
			}
			children.sort((a, b) => (a.hints?.order ?? 9999) - (b.hints?.order ?? 9999))
			break
		}

		case 'lazy': {
			widget = 'json'
			break
		}

		case 'union': {
			// Check if this is a simple nullable/optional union (e.g. z.union([z.null(), ActualSchema]))
			const unionOpts = flattenUnionOptions(inner)
			const nonNullOpts = unionOpts.filter(
				(opt: z.ZodType) =>
					unwrapZodType(opt).typeName !== 'null' &&
					unwrapZodType(opt).typeName !== 'undefined'
			)
			const hasNull = unionOpts.length > nonNullOpts.length
			if (hasNull) {
				isNullable = true
			}

			const singleOptType =
				nonNullOpts.length === 1 ? unwrapZodType(nonNullOpts[0]).typeName : ''
			const isNullableObjectUnion =
				hasNull &&
				nonNullOpts.length === 1 &&
				(singleOptType === 'object' || singleOptType === 'intersection')

			if (nonNullOpts.length === 1 && !isNullableObjectUnion) {
				const innerDesc = introspectField(key, nonNullOpts[0], basePath, hints)
				const isOpt = isOptional || hasNull
				const isNull = hasNull || isNullable
				return {
					...innerDesc,
					isOptional: isOpt || innerDesc.isOptional,
					isNullable: isNull || innerDesc.isNullable,
					required: isOpt || isNull ? false : innerDesc.required
				}
			}

			// Check if this is a Sensitive secret schema (z.union([z.string(), z.object({ set: z.string() })]))
			if (isSensitiveSchema(inner) || hint.widget === 'password' || hint.sensitive) {
				widget = 'password'
				sensitive = true
				break
			}

			// Check if this is an AttributeFilter schema ({ and: [...] } | { or: [...] } | { match: ... })
			if (isAttributeFilterSchema(inner) || hint.widget === 'attribute-filter') {
				widget = 'attribute-filter'
				defaultValue = {
					match: {
						key: '',
						key_is_regex: false,
						op: '=',
						table: 'record',
						value: { string: '' }
					}
				}
				break
			}

			const resolved = resolveUnion(inner, path, hints, hasNull || isNullable || isOptional)
			if (resolved.options.length > 0) {
				widget = 'discriminated-union'
				unionDiscriminator = resolved.discriminator
				unionTagMode = resolved.tagMode
				options = resolved.options
				unionBranches = resolved.branches
				if (isNullableObjectUnion && resolved.branches.enabled) {
					children = resolved.branches.enabled
				}
			} else {
				widget = 'json'
			}
			break
		}

		default:
			widget = 'text'
	}

	// Override and enhancements from UI hints
	if (hint.format) {
		format = hint.format
	}
	if (hint.rules && Array.isArray(hint.rules)) {
		for (const r of hint.rules) {
			if (!rules.includes(r)) rules.push(r)
		}
	}

	return {
		name: key,
		path,
		pathKey,
		label: hint.label || formatFieldLabel(key),
		widget: hint.widget || widget,
		sensitive: hint.sensitive ?? sensitive,
		zodType: typeName,
		required: !isOptional && !isNullable && !unwrapped.hasDefault,
		isOptional,
		isNullable,
		defaultValue:
			hint.defaultValue !== undefined
				? hint.defaultValue
				: defaultValue !== undefined
					? defaultValue
					: unwrapped.defaultValue !== undefined
						? unwrapped.defaultValue
						: unwrapped.typeName === 'record'
							? {}
							: undefined,
		description: unwrapped.description || hint.description,
		placeholder: hint.placeholder,
		min,
		max,
		step,
		format,
		rules: rules.length > 0 ? rules : undefined,
		pattern,
		regex,
		options:
			hint.keyOptions && Array.isArray(hint.keyOptions)
				? hint.keyOptions
				: hint.options && Array.isArray(hint.options)
					? hint.options
					: options,
		children,
		unionDiscriminator,
		unionTagMode,
		unionBranches,
		hints: hint
	}
}

/**
 * Introspects all properties of a ZodObject into an array of FieldDescriptors
 */
export function introspectObjectSchema(
	objectSchema: z.ZodObject<any>,
	basePath: FieldPath = [],
	hints: Record<string, UiHint> = {}
): FieldDescriptor[] {
	const def = getZodDef(objectSchema)
	const shape =
		objectSchema.shape ?? (typeof def?.shape === 'function' ? def.shape() : def?.shape) ?? {}
	const descriptors: FieldDescriptor[] = []

	for (const [key, fieldSchema] of Object.entries(shape)) {
		descriptors.push(introspectField(key, fieldSchema as z.ZodType, basePath, hints))
	}

	// Sort fields based on hint.order property if specified
	descriptors.sort((a, b) => {
		const orderA = a.hints?.order ?? 9999
		const orderB = b.hints?.order ?? 9999
		return orderA - orderB
	})

	return descriptors
}

/**
 * Main entry point for runtime Zod schema introspection
 */
export function introspectZodSchema(
	schema: z.ZodType,
	basePath: FieldPath = [],
	hints: Record<string, UiHint> = {}
): FieldDescriptor[] {
	const unwrapped = unwrapZodType(schema)
	if (unwrapped.typeName === 'object') {
		return introspectObjectSchema(unwrapped.innerType as z.ZodObject<any>, basePath, hints)
	}
	if (unwrapped.typeName === 'intersection') {
		const shape = extractObjectShape(unwrapped.innerType)
		const descriptors = Object.entries(shape).map(([key, fieldSchema]) =>
			introspectField(key, fieldSchema, basePath, hints)
		)
		descriptors.sort((a, b) => (a.hints?.order ?? 9999) - (b.hints?.order ?? 9999))
		return descriptors
	}
	return [introspectField('root', unwrapped.innerType, basePath, hints)]
}

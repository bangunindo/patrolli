// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { z } from 'zod'
import type { FieldDescriptor, FieldPath, UiHint } from '@@/shared/types/dynamic-form.type'
import { formatFieldLabel, pathToString } from './path-helper'

export interface UnwrappedZodType {
	innerType: z.ZodTypeAny
	typeName: string
	isOptional: boolean
	isNullable: boolean
	hasDefault: boolean
	defaultValue?: any
	description?: string
}

/**
 * Recursively unwraps Zod wrappers (Optional, Nullable, Default, Effects)
 * to extract the inner schema and constraint metadata.
 */
export function unwrapZodType(schema: z.ZodTypeAny | undefined | null): UnwrappedZodType {
	if (!schema) {
		return {
			innerType: undefined as any,
			typeName: 'Unknown',
			isOptional: true,
			isNullable: true,
			hasDefault: false,
			defaultValue: undefined,
			description: undefined
		}
	}

	let current: z.ZodTypeAny = schema
	let isOptional = false
	let isNullable = false
	let hasDefault = false
	let defaultValue: any = undefined
	let description: string | undefined = schema.description

	while (current) {
		const typeName = current._def?.typeName
		description = current.description || description

		if (typeName === 'ZodOptional') {
			isOptional = true
			current = (current as z.ZodOptional<any>)._def.innerType
		} else if (typeName === 'ZodNullable') {
			isNullable = true
			current = (current as z.ZodNullable<any>)._def.innerType
		} else if (typeName === 'ZodDefault') {
			hasDefault = true
			try {
				defaultValue = (current as z.ZodDefault<any>)._def.defaultValue()
			} catch {
				defaultValue = undefined
			}
			current = (current as z.ZodDefault<any>)._def.innerType
		} else if (typeName === 'ZodEffects') {
			current = (current as z.ZodEffects<any>)._def.schema
		} else if (typeName === 'ZodLazy') {
			// Recursive schema: stop unwrapping to prevent infinite recursion
			return {
				innerType: current,
				typeName: 'ZodLazy',
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
		typeName: current?._def?.typeName || 'Unknown',
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
export function extractLiteralValue(schema: z.ZodTypeAny): string | undefined {
	const unwrapped = unwrapZodType(schema)
	if (unwrapped.typeName === 'ZodLiteral') {
		return (unwrapped.innerType as z.ZodLiteral<any>)._def.value
	}
	if (unwrapped.typeName === 'ZodEnum') {
		return (unwrapped.innerType as z.ZodEnum<any>)._def.values[0]
	}
	return undefined
}

/**
 * Extracts object shape from ZodObject or ZodIntersection
 */
export function extractObjectShape(schema: z.ZodTypeAny): Record<string, z.ZodTypeAny> {
	const unwrapped = unwrapZodType(schema)
	if (unwrapped.typeName === 'ZodObject') {
		const obj = unwrapped.innerType as z.ZodObject<any>
		return typeof obj._def.shape === 'function' ? obj._def.shape() : obj.shape
	}
	if (unwrapped.typeName === 'ZodIntersection') {
		const inter = unwrapped.innerType as z.ZodIntersection<any, any>
		const leftShape = extractObjectShape(inter._def.left)
		const rightShape = extractObjectShape(inter._def.right)
		return { ...leftShape, ...rightShape }
	}
	return {}
}

/**
 * Checks if a Zod schema is an AttributeFilter ({ and: [...] } | { or: [...] } | { match: ... })
 */
export function isAttributeFilterSchema(schema: z.ZodTypeAny): boolean {
	const unwrapped = unwrapZodType(schema)
	if (unwrapped.typeName !== 'ZodUnion') return false
	const options = (unwrapped.innerType as any)._def?.options || []
	if (options.length < 3) return false

	let hasAnd = false
	let hasOr = false
	let hasMatch = false

	for (const opt of options) {
		const optUnwrapped = unwrapZodType(opt)
		if (optUnwrapped.typeName === 'ZodObject') {
			const shape = extractObjectShape(optUnwrapped.innerType)
			if ('and' in shape) hasAnd = true
			if ('or' in shape) hasOr = true
			if ('match' in shape) hasMatch = true
		}
	}

	return hasAnd && hasOr && hasMatch
}

export function flattenUnionOptions(schema: z.ZodTypeAny): z.ZodTypeAny[] {
	const unwrapped = unwrapZodType(schema)
	const rawOptions =
		(unwrapped.innerType as any)._def?.options || (unwrapped.innerType as any).options || []
	const result: z.ZodTypeAny[] = []
	for (const opt of rawOptions) {
		const optUnwrapped = unwrapZodType(opt)
		if (optUnwrapped.typeName === 'ZodUnion') {
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
	unionSchema: z.ZodTypeAny,
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
		(opt: z.ZodTypeAny) =>
			unwrapZodType(opt).typeName !== 'ZodNull' &&
			unwrapZodType(opt).typeName !== 'ZodUndefined'
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
			nonNullOptions.every((opt: z.ZodTypeAny) => {
				const unwrapped = unwrapZodType(opt)
				if (
					unwrapped.typeName === 'ZodObject' ||
					unwrapped.typeName === 'ZodIntersection'
				) {
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
			if (unwrapped.typeName === 'ZodObject' || unwrapped.typeName === 'ZodIntersection') {
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

	// 2. Check if this is an externally-tagged union:
	// Each branch is a ZodObject with exactly 1 property (variant name)
	const isExternalTagged =
		nonNullOptions.length > 0 &&
		nonNullOptions.every((opt: z.ZodTypeAny) => {
			const unwrapped = unwrapZodType(opt)
			if (unwrapped.typeName === 'ZodObject' || unwrapped.typeName === 'ZodIntersection') {
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
	rawSchema: z.ZodTypeAny,
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

	switch (typeName) {
		case 'ZodString': {
			widget = hint.sensitive ? 'password' : 'text'
			const checks = (inner as z.ZodString)._def.checks || []
			for (const check of checks) {
				if (check.kind === 'min') min = check.value
				if (check.kind === 'max') max = check.value
				if (check.kind === 'uuid') {
					format = 'uuid'
					if (!rules.includes('uuid')) rules.push('uuid')
				}
				if (check.kind === 'email') {
					format = 'email'
					if (!rules.includes('email')) rules.push('email')
				}
				if (check.kind === 'ip') {
					format = 'ip'
					if (!rules.includes('ip')) rules.push('ip')
				}
				if (check.kind === 'url') {
					format = 'url'
					if (!rules.includes('url')) rules.push('url')
				}
				if (check.kind === 'regex') {
					regex = check.regex
					pattern = check.regex.source
				}
			}
			break
		}

		case 'ZodNumber': {
			widget = 'number'
			const checks = (inner as z.ZodNumber)._def.checks || []
			for (const check of checks) {
				if (check.kind === 'min') min = check.value
				if (check.kind === 'max') max = check.value
				if (check.kind === 'int') step = 1
			}
			break
		}

		case 'ZodBoolean': {
			widget = 'switch'
			break
		}

		case 'ZodEnum': {
			widget = 'select'
			const values = (inner as z.ZodEnum<any>)._def.values || []
			options = values.map((val: string) => ({
				label: formatFieldLabel(val),
				value: val
			}))
			break
		}

		case 'ZodNativeEnum': {
			widget = 'select'
			const enumObj = (inner as z.ZodNativeEnum<any>)._def.values
			options = Object.keys(enumObj)
				.filter((k) => isNaN(Number(k)))
				.map((k) => ({
					label: formatFieldLabel(k),
					value: enumObj[k]
				}))
			break
		}

		case 'ZodArray': {
			defaultValue = []
			const elementSchema = (inner as z.ZodArray<any>)._def.type
			const elemUnwrapped = unwrapZodType(elementSchema)
			if (elemUnwrapped.typeName === 'ZodString') {
				widget = 'array-string'
			} else if (elemUnwrapped.typeName === 'ZodObject') {
				widget = 'array-object'
				children = introspectObjectSchema(
					elemUnwrapped.innerType as z.ZodObject<any>,
					path,
					hints
				)
			} else {
				widget = 'array-string'
			}
			break
		}

		case 'ZodRecord': {
			defaultValue = {}
			widget = 'record'
			const rec = inner as z.ZodRecord<any, any>
			const keySchema = rec._def?.keyType || (rec as any).keySchema
			const valueSchema = rec._def?.valueType || (rec as any).valueSchema
			if (keySchema) {
				const keyUnwrapped = unwrapZodType(keySchema)
				if (keyUnwrapped.typeName === 'ZodEnum') {
					const values = (keyUnwrapped.innerType as z.ZodEnum<any>)._def.values || []
					options = values.map((val: string) => ({
						label: formatFieldLabel(val),
						value: val
					}))
				}
			}
			if (valueSchema) {
				children = [introspectField('value', valueSchema, path, hints)]
			}
			break
		}

		case 'ZodUnknown':
		case 'ZodAny': {
			widget = 'json'
			break
		}

		case 'ZodObject': {
			widget = 'custom'
			children = introspectObjectSchema(inner as z.ZodObject<any>, path, hints)
			break
		}

		case 'ZodIntersection': {
			widget = 'custom'
			const shape = extractObjectShape(inner)
			children = []
			for (const [propKey, propSchema] of Object.entries(shape)) {
				children.push(introspectField(propKey, propSchema, path, hints))
			}
			break
		}

		case 'ZodLazy': {
			widget = 'json'
			break
		}

		case 'ZodUnion':
		case 'ZodDiscriminatedUnion': {
			// Check if this is a simple nullable/optional union (e.g. z.union([z.null(), ActualSchema]))
			const unionOpts = flattenUnionOptions(inner)
			const nonNullOpts = unionOpts.filter(
				(opt: z.ZodTypeAny) =>
					unwrapZodType(opt).typeName !== 'ZodNull' &&
					unwrapZodType(opt).typeName !== 'ZodUndefined'
			)
			const hasNull = unionOpts.length > nonNullOpts.length
			if (hasNull) {
				isNullable = true
			}

			if (nonNullOpts.length === 1) {
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
						: unwrapped.typeName === 'ZodRecord'
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
	const shape =
		typeof objectSchema._def.shape === 'function'
			? objectSchema._def.shape()
			: objectSchema.shape
	const descriptors: FieldDescriptor[] = []

	for (const [key, fieldSchema] of Object.entries(shape)) {
		descriptors.push(introspectField(key, fieldSchema as z.ZodTypeAny, basePath, hints))
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
	schema: z.ZodTypeAny,
	basePath: FieldPath = [],
	hints: Record<string, UiHint> = {}
): FieldDescriptor[] {
	const unwrapped = unwrapZodType(schema)
	if (unwrapped.typeName === 'ZodObject') {
		return introspectObjectSchema(unwrapped.innerType as z.ZodObject<any>, basePath, hints)
	}
	return [introspectField('root', unwrapped.innerType, basePath, hints)]
}

// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { toRaw } from 'vue'
import { z } from 'zod'
import {
	unwrapZodType,
	extractLiteralValue,
	extractObjectShape,
	flattenUnionOptions
} from './zod-introspector'
import { pathToString, formatFieldLabel, getValueByPath } from './path-helper'
import { validateFormat, type ValidationContext } from './format-registry'
import type { FormValidationResult, UiHint } from '@@/shared/types/dynamic-form.type'

/**
 * Formats a ZodIssue into a user-friendly error message
 */
export function humanizeZodIssue(issue: z.ZodIssue, hints: Record<string, UiHint> = {}): string {
	const pathKey = issue.path.join('.')
	const hint = hints[pathKey] || {}
	if (hint.errorMessage) {
		return hint.errorMessage
	}

	const lastSegment = issue.path[issue.path.length - 1]
	const fieldLabel =
		hint.label || (typeof lastSegment === 'string' ? formatFieldLabel(lastSegment) : 'Field')

	switch (issue.code) {
		case 'invalid_string': {
			if (issue.validation === 'uuid') {
				return `${fieldLabel} must be a valid UUID`
			}
			if (issue.validation === 'email') {
				return `${fieldLabel} must be a valid email address`
			}
			if (issue.validation === 'ip') {
				return `${fieldLabel} must be a valid IP address`
			}
			if (issue.validation === 'url') {
				return `${fieldLabel} must be a valid URL`
			}
			if (issue.validation === 'regex') {
				return `${fieldLabel} format is invalid`
			}
			return `${fieldLabel} is invalid: ${issue.message}`
		}

		case 'invalid_type': {
			if (issue.received === 'undefined' || issue.received === 'null') {
				return `${fieldLabel} is required`
			}
			return `${fieldLabel} must be of type ${issue.expected}`
		}

		case 'too_small': {
			if (issue.type === 'string') {
				if (issue.minimum === 1) {
					return `${fieldLabel} cannot be empty`
				}
				return `${fieldLabel} must be at least ${issue.minimum} characters`
			}
			if (issue.type === 'number') {
				return `${fieldLabel} must be greater than or equal to ${issue.minimum}`
			}
			if (issue.type === 'array') {
				return `${fieldLabel} cannot be empty`
			}
			return `${fieldLabel} is too small`
		}

		case 'too_big': {
			if (issue.type === 'string') {
				return `${fieldLabel} must be at most ${issue.maximum} characters`
			}
			if (issue.type === 'number') {
				return `${fieldLabel} must be less than or equal to ${issue.maximum}`
			}
			return `${fieldLabel} is too large`
		}

		default:
			return issue.message || `${fieldLabel} is invalid`
	}
}

interface ExtractedIssue {
	path: (string | number)[]
	message: string
	issue: z.ZodIssue
}

/**
 * Extracts the discriminator value from data object
 */
function extractDiscriminatorValue(
	data: any,
	preferredKey?: string
): { key: string; val: any } | null {
	if (!data || typeof data !== 'object') return null
	const candidateKeys = preferredKey
		? [preferredKey, 'type', 'module', 'kind', 'method', 'strategy']
		: ['type', 'module', 'kind', 'method', 'strategy']

	for (const k of candidateKeys) {
		if (k in data && data[k] !== undefined && data[k] !== null) {
			return { key: k, val: data[k] }
		}
	}
	return null
}

/**
 * Finds the ZodError from the union branch that best matches the input data,
 * handling internal-tagged, external-tagged, and nullable wrapper unions recursively.
 */
function findMatchingUnionError(
	unionErrors: z.ZodError[],
	issuePath: (string | number)[],
	unionData: any,
	preferredKey?: string
): z.ZodError | undefined {
	if (!unionData || typeof unionData !== 'object') {
		return undefined
	}

	const disc = extractDiscriminatorValue(unionData, preferredKey)
	const prefixLen = issuePath.length

	// 1. Internal tagged: match based on discriminator key & value at this error level
	if (disc && disc.val !== undefined) {
		const matched = unionErrors.find((err) => {
			const hasDiscMismatch = err.issues.some((sub) => {
				const isExactDiscKey =
					sub.path.length === prefixLen + 1 && sub.path[prefixLen] === disc.key
				return (
					isExactDiscKey &&
					(sub.code === 'invalid_type' ||
						sub.code === 'invalid_literal' ||
						sub.code === 'invalid_enum_value')
				)
			})
			return !hasDiscMismatch
		})
		if (matched) return matched
	}

	// 2. Unnest wrapper unions (e.g. z.union([z.null(), ActualUnion]))
	// Only unnest if issue occurs exactly at the issuePath level
	const flatErrors: z.ZodError[] = []
	for (const err of unionErrors) {
		const isNullMismatch =
			err.issues.length === 1 &&
			err.issues[0].code === 'invalid_type' &&
			(err.issues[0].expected === 'null' || err.issues[0].expected === 'undefined')
		if (isNullMismatch) {
			continue
		}

		if (
			err.issues.length === 1 &&
			err.issues[0].code === 'invalid_union' &&
			Array.isArray((err.issues[0] as any).unionErrors) &&
			err.issues[0].path.length === prefixLen
		) {
			const nestedErrors: z.ZodError[] = (err.issues[0] as any).unionErrors
			const matchedNested = findMatchingUnionError(
				nestedErrors,
				issuePath,
				unionData,
				preferredKey
			)
			if (matchedNested) {
				return matchedNested
			}
			flatErrors.push(...nestedErrors)
		} else {
			flatErrors.push(err)
		}
	}

	const candidates = flatErrors.length > 0 ? flatErrors : unionErrors

	// 3. External tagged: branch matches if there is an issue in the segment right after issuePath that matches key in unionData
	const externalMatched = candidates.find((err) => {
		return err.issues.some((sub) => {
			const variantKey = sub.path[prefixLen]
			return typeof variantKey === 'string' && variantKey in unionData
		})
	})
	if (externalMatched) return externalMatched

	return undefined
}

/**
 * Extracts ZodIssues from unionErrors recursively based on the active branch
 */
function flattenZodIssues(
	issues: z.ZodIssue[],
	rootData: any,
	hints: Record<string, UiHint> = {}
): ExtractedIssue[] {
	const result: ExtractedIssue[] = []

	for (const issue of issues) {
		if (issue.code === 'invalid_union' && Array.isArray((issue as any).unionErrors)) {
			const unionErrors: z.ZodError[] = (issue as any).unionErrors
			const pathKey = issue.path.join('.')
			const parentHint = hints[pathKey] || {}
			const unionData = getValueByPath(rootData, issue.path as string[])

			const chosenError =
				findMatchingUnionError(
					unionErrors,
					issue.path,
					unionData,
					parentHint.discriminatorField
				) ||
				(unionErrors.length > 0
					? [...unionErrors].sort((a, b) => a.issues.length - b.issues.length)[0]
					: undefined)

			if (chosenError && chosenError.issues.length > 0) {
				const subFlattened = flattenZodIssues(chosenError.issues, rootData, hints)
				for (const sub of subFlattened) {
					const isPrefixed =
						issue.path.length > 0 &&
						issue.path.every((seg, idx) => String(sub.path[idx]) === String(seg))
					const fullPath = isPrefixed ? sub.path : [...issue.path, ...sub.path]
					result.push({
						path: fullPath,
						message: sub.message,
						issue: sub.issue
					})
				}
				continue
			}
		}

		result.push({
			path: issue.path,
			message: humanizeZodIssue(issue, hints),
			issue
		})
	}

	return result
}

function resolveBranchAliasPath(root: any, path: string[]): string[] | null {
	for (let i = 0; i < path.length - 1; i++) {
		const sub = getValueByPath(root, path.slice(0, i + 1))
		if (sub && typeof sub === 'object') {
			const disc = extractDiscriminatorValue(sub)
			if (disc && disc.val) {
				if (disc.val === path[i + 1]) {
					if ('config' in sub) {
						const altPath = [...path.slice(0, i + 1), 'config', ...path.slice(i + 2)]
						if (getValueByPath(root, altPath) !== undefined) {
							return altPath
						}
					}
					const altPath2 = [...path.slice(0, i + 1), ...path.slice(i + 2)]
					if (getValueByPath(root, altPath2) !== undefined) {
						return altPath2
					}
				}
				if (path[i + 1] === 'config') {
					return [...path.slice(0, i + 1), String(disc.val), ...path.slice(i + 2)]
				}
			}
		}
	}
	return null
}

function generatePathAliases(root: any, path: (string | number)[]): string[] {
	const aliases: string[] = []
	const pathStr = path.map(String)
	for (let i = 0; i < pathStr.length - 1; i++) {
		const sub = getValueByPath(root, pathStr.slice(0, i + 1))
		if (sub && typeof sub === 'object') {
			const disc = extractDiscriminatorValue(sub)
			if (disc && disc.val && pathStr[i + 1] === 'config') {
				aliases.push(
					[...pathStr.slice(0, i + 1), String(disc.val), ...pathStr.slice(i + 2)].join(
						'.'
					)
				)
				aliases.push(
					[
						...pathStr.slice(0, i + 1),
						String(disc.val),
						'config',
						...pathStr.slice(i + 2)
					].join('.')
				)
			}
		}
	}
	return aliases
}

function safeClone<T>(obj: T): T {
	if (!obj || typeof obj !== 'object') return obj
	try {
		return structuredClone(toRaw(obj))
	} catch {
		try {
			return JSON.parse(JSON.stringify(obj))
		} catch {
			return { ...obj }
		}
	}
}

function registerIssueWithAliases(
	issues: Record<string, string>,
	key: string,
	msg: string,
	root: any
) {
	issues[key] = msg
	const parts = key.split('.')
	if (parts[0] === 'config' && parts[1] === 'config') {
		const activeType = root?.config?.type || root?.type
		if (activeType) {
			const branchAlias = ['config', activeType, ...parts.slice(2)].join('.')
			if (!issues[branchAlias]) issues[branchAlias] = msg
		}
	} else if (parts[0] === 'config' && parts.length > 2 && parts[1] !== 'config') {
		const configAlias = ['config', 'config', ...parts.slice(2)].join('.')
		if (!issues[configAlias]) issues[configAlias] = msg
	}
	const aliases = generatePathAliases(root, parts)
	for (const alias of aliases) {
		if (!issues[alias]) issues[alias] = msg
	}
}

export class AutoFormAdapter {
	/**
	 * Sanitizes draft data and validates against Zod schema and format registry
	 */
	static toRequest<T>(
		schema: z.ZodType<T>,
		draft: Record<string, any> | undefined | null,
		hints: Record<string, UiHint> = {}
	): FormValidationResult<T> {
		const draftCopy = safeClone(draft ?? {})
		const sanitized = this.deepSanitize(draftCopy, schema, [], hints, draftCopy)
		const issues: Record<string, string> = {}

		// 1. Run format validation and custom validators from hints
		for (const [pathKey, hint] of Object.entries(hints)) {
			const path = pathKey.split('.')
			let value = getValueByPath(sanitized, path)
			if (value === undefined && path.length > 2) {
				const resolvedPath = resolveBranchAliasPath(sanitized, path)
				if (resolvedPath) {
					value = getValueByPath(sanitized, resolvedPath)
				}
			}
			const label = hint.label || formatFieldLabel(path[path.length - 1])
			const context: ValidationContext = {
				path,
				pathKey,
				label,
				draft: sanitized
			}

			if (hint.format && value !== undefined && value !== null && value !== '') {
				const errorMsg = validateFormat(hint.format, value, context)
				if (errorMsg) {
					registerIssueWithAliases(
						issues,
						pathKey,
						hint.errorMessage || errorMsg,
						sanitized
					)
				}
			}

			if (hint.validator) {
				try {
					const res = hint.validator(value, sanitized)
					if (typeof res === 'string' && res.length > 0) {
						registerIssueWithAliases(issues, pathKey, res, sanitized)
					} else if (res && typeof res === 'object') {
						for (const [k, msg] of Object.entries(res)) {
							registerIssueWithAliases(issues, k, msg as string, sanitized)
						}
					} else if (res === false) {
						registerIssueWithAliases(
							issues,
							pathKey,
							hint.errorMessage || `${label} is invalid`,
							sanitized
						)
					}
				} catch (err: any) {
					registerIssueWithAliases(
						issues,
						pathKey,
						err?.message || `${label} validation error`,
						sanitized
					)
				}
			}
		}

		// 2. Run Zod schema validation
		const result = schema.safeParse(sanitized)

		if (!result.success) {
			const flattened = flattenZodIssues(result.error.issues, sanitized, hints)

			for (const item of flattened) {
				const pathKey = item.path.join('.')
				if (!issues[pathKey]) {
					issues[pathKey] = item.message
				}

				// Register path aliases for nested discriminated unions
				const aliases = generatePathAliases(sanitized, item.path)
				for (const alias of aliases) {
					if (!issues[alias]) {
						issues[alias] = item.message
					}
				}

				// If the path contains a numeric array index (e.g. 'config.hosts.0'),
				// attach the issue to the parent container ('config.hosts') so container widgets can detect it
				const lastSeg = item.path[item.path.length - 1]
				if (typeof lastSeg === 'number' && item.path.length > 1) {
					const parentPathKey = item.path.slice(0, -1).join('.')
					if (!issues[parentPathKey]) {
						issues[parentPathKey] = item.message
					}
				}
			}
		}

		if (Object.keys(issues).length > 0) {
			return { success: false, issues }
		}

		return { success: true, data: result.success ? result.data : (sanitized as T) }
	}

	/**
	 * Compatibility wrapper for schema sanitization and validation
	 */
	static sanitizeAndValidate<T>(
		schema: z.ZodType<T>,
		draft: Record<string, any> | undefined | null,
		hints: Record<string, UiHint> = {}
	): { isValid: boolean; data?: T; issues: Record<string, string>; errors: string[] } {
		const res = this.toRequest(schema, draft, hints)
		return {
			isValid: res.success,
			data: res.success ? res.data : undefined,
			issues: !res.success ? res.issues : {},
			errors: !res.success ? Object.values(res.issues) : []
		}
	}

	/**
	 * Helper to look up an issue in issues map with flexible path matching
	 * (exact match, relative match, or suffix match for nested forms)
	 */
	static findIssue(issues: Record<string, string>, targetKey: string): string | null {
		if (!issues || Object.keys(issues).length === 0) return null

		// 1. Exact match
		if (issues[targetKey]) return issues[targetKey]

		// 2. If targetKey ends with field name (e.g. 'address' or 'listener.address')
		const dotTarget = '.' + targetKey
		for (const [key, msg] of Object.entries(issues)) {
			if (key.endsWith(dotTarget)) {
				return msg
			}
		}

		// 3. Check for last 2 segment partial match
		const targetParts = targetKey.split('.')
		if (targetParts.length > 1) {
			const targetLast2 = targetParts.slice(-2).join('.')
			for (const [key, msg] of Object.entries(issues)) {
				if (key.endsWith('.' + targetLast2)) {
					return msg
				}
			}
		}

		return null
	}

	/**
	 * Validates a specific single field, supporting absolute paths or nested field identifiers
	 */
	static validateField(
		schema: z.ZodTypeAny,
		fieldPathKey: string,
		draft: Record<string, any>,
		hints: Record<string, UiHint> = {}
	): string | null {
		const res = this.toRequest(schema, draft, hints)
		if (res.success) return null

		// If fieldPathKey is a root field and schema defines it at root level,
		// do not match nested fields sharing the same name (e.g. root worker_count vs listener.worker_count)
		if (!fieldPathKey.includes('.')) {
			const unwrapped = unwrapZodType(schema)
			if (unwrapped.typeName === 'ZodObject') {
				const shape = extractObjectShape(unwrapped.innerType)
				if (fieldPathKey in shape) {
					return res.issues[fieldPathKey] || null
				}
			}
		}

		return this.findIssue(res.issues, fieldPathKey)
	}

	/**
	 * Validates a specific sub-scope or nested section (e.g. 'config', 'config.listener', 'channel')
	 * Returns matching issues within that scope.
	 */
	static validateScope(
		schema: z.ZodTypeAny,
		scopePath: string | string[],
		draft: Record<string, any>,
		hints: Record<string, UiHint> = {}
	): Record<string, string> {
		const res = this.toRequest(schema, draft, hints)
		if (res.success) return {}

		const scopeKey = Array.isArray(scopePath) ? scopePath.join('.') : scopePath
		const matched: Record<string, string> = {}
		const dotScope = '.' + scopeKey

		for (const [key, msg] of Object.entries(res.issues)) {
			if (key === scopeKey || key.startsWith(scopeKey + '.')) {
				matched[key] = msg
			} else if (key.includes(dotScope + '.')) {
				matched[key] = msg
			} else {
				// Check segment matching
				const keySegs = key.split('.')
				const scopeSegs = scopeKey.split('.')
				const idx = keySegs.indexOf(scopeSegs[0])
				if (idx >= 0) {
					const subSegs = keySegs.slice(idx, idx + scopeSegs.length).join('.')
					if (subSegs === scopeKey) {
						matched[key] = msg
					}
				}
			}
		}

		return matched
	}

	/**
	 * Prepares initial draft state from backend entity, filling default values where absent
	 */
	static toDraft(
		schema: z.ZodTypeAny,
		entity?: Record<string, any> | null,
		hints: Record<string, UiHint> = {},
		currentPath: string[] = [],
		rootDraft?: Record<string, any>
	): Record<string, any> {
		let draft: Record<string, any> = {}
		if (entity) {
			try {
				draft = structuredClone(toRaw(entity))
			} catch {
				draft = JSON.parse(JSON.stringify(entity))
			}
		}
		const root = rootDraft || draft
		const unwrapped = unwrapZodType(schema)

		if (unwrapped.typeName === 'ZodObject' || unwrapped.typeName === 'ZodIntersection') {
			const shape = extractObjectShape(unwrapped.innerType)

			for (const [key, fieldSchema] of Object.entries(shape)) {
				const fieldPath = [...currentPath, key]
				const fieldPathKey = pathToString(fieldPath)
				const fieldUnwrapped = unwrapZodType(fieldSchema as z.ZodTypeAny)

				let fieldHint: UiHint | undefined
				// Check specific discriminator branch alias (e.g. config.open_telemetry.listener.address)
				if (currentPath.length >= 1 && currentPath[0] === 'config') {
					const activeType = root.config?.type || root.type
					if (activeType) {
						const subSegments =
							currentPath[1] === 'config'
								? currentPath.slice(2)
								: currentPath.slice(1)
						const branchKey = ['config', activeType, ...subSegments, key].join('.')
						fieldHint = hints[branchKey]
					}
				}
				if (!fieldHint) {
					fieldHint = hints[fieldPathKey]
				}
				if (!fieldHint) {
					const dotKey = '.' + key
					const activeType = root?.config?.type || root?.type
					for (const [hk, hv] of Object.entries(hints)) {
						const hkParts = hk.split('.')
						if (hkParts.length > 2 && hkParts[0] === 'config' && activeType) {
							const branchName = hkParts[1]
							if (branchName !== 'config' && branchName !== activeType) {
								continue
							}
						}
						if (hk === key || hk.endsWith(dotKey)) {
							fieldHint = hv
							break
						}
					}
				}
				fieldHint = fieldHint || {}
				if (
					key === 'labels' ||
					fieldHint.widget === 'label' ||
					fieldHint.emptyPolicy === 'empty-object'
				) {
					if (draft[key] === undefined || draft[key] === null || draft[key] === '') {
						draft[key] = JSON.stringify({})
					} else if (typeof draft[key] === 'object') {
						draft[key] = JSON.stringify(draft[key])
					}
				} else if (draft[key] === undefined) {
					if (fieldHint.defaultValue !== undefined) {
						draft[key] = structuredClone(fieldHint.defaultValue)
					} else if (fieldUnwrapped.hasDefault) {
						draft[key] = fieldUnwrapped.defaultValue
					} else if (fieldUnwrapped.typeName === 'ZodLiteral') {
						draft[key] = (fieldUnwrapped.innerType as any)._def.value
					} else if (fieldUnwrapped.typeName === 'ZodEnum') {
						const enumValues = (fieldUnwrapped.innerType as any)._def.values || []
						if (enumValues.length > 0) {
							draft[key] = enumValues[0]
						}
					} else if (fieldUnwrapped.typeName === 'ZodBoolean') {
						if (!fieldUnwrapped.isOptional && !fieldUnwrapped.isNullable) {
							draft[key] = false
						}
					} else if (
						fieldUnwrapped.typeName === 'ZodUnion' ||
						fieldUnwrapped.typeName === 'ZodDiscriminatedUnion'
					) {
						// Initialize matching branch: check if discriminatorDefault exists in hints
						const rawOptions = flattenUnionOptions(fieldUnwrapped.innerType)
						const hasNull = rawOptions.some(
							(opt) =>
								unwrapZodType(opt).typeName === 'ZodNull' ||
								unwrapZodType(opt).typeName === 'ZodUndefined'
						)
						const nonNullOptions = rawOptions.filter(
							(opt) =>
								unwrapZodType(opt).typeName !== 'ZodNull' &&
								unwrapZodType(opt).typeName !== 'ZodUndefined'
						)

						if (hasNull || fieldUnwrapped.isOptional || fieldUnwrapped.isNullable) {
							// If a specific default discriminator is requested in hints (other than 'none')
							if (
								fieldHint.discriminatorDefault &&
								fieldHint.discriminatorDefault !== 'none' &&
								nonNullOptions.length > 0
							) {
								const targetOpt = nonNullOptions.find((opt) => {
									const shape = extractObjectShape(opt)
									const discVal =
										extractLiteralValue(shape['type']) ??
										extractLiteralValue(shape['module']) ??
										extractLiteralValue(shape['kind'])
									return discVal === fieldHint.discriminatorDefault
								})
								draft[key] = this.toDraft(
									targetOpt || nonNullOptions[0],
									null,
									hints,
									fieldPath,
									root
								)
							} else {
								draft[key] = null
							}
						} else if (nonNullOptions.length > 0) {
							const targetOpt = fieldHint.discriminatorDefault
								? nonNullOptions.find((opt) => {
										const shape = extractObjectShape(opt)
										const discVal =
											extractLiteralValue(shape['type']) ??
											extractLiteralValue(shape['module']) ??
											extractLiteralValue(shape['kind'])
										return discVal === fieldHint.discriminatorDefault
									})
								: undefined
							draft[key] = this.toDraft(
								targetOpt || nonNullOptions[0],
								null,
								hints,
								fieldPath,
								root
							)
						}
					} else if (
						fieldUnwrapped.typeName === 'ZodObject' ||
						fieldUnwrapped.typeName === 'ZodIntersection'
					) {
						draft[key] = this.toDraft(
							fieldSchema as z.ZodTypeAny,
							null,
							hints,
							fieldPath,
							root
						)
					}
				} else if (
					draft[key] !== null &&
					typeof draft[key] === 'object' &&
					!Array.isArray(draft[key])
				) {
					if (
						fieldUnwrapped.typeName === 'ZodUnion' ||
						fieldUnwrapped.typeName === 'ZodDiscriminatedUnion'
					) {
						// Find option matching current discriminator
						const rawOptions = flattenUnionOptions(fieldUnwrapped.innerType)
						const discKey =
							'type' in draft[key]
								? 'type'
								: 'module' in draft[key]
									? 'module'
									: 'kind' in draft[key]
										? 'kind'
										: 'method' in draft[key]
											? 'method'
											: 'strategy' in draft[key]
												? 'strategy'
												: undefined
						if (discKey && draft[key][discKey]) {
							const matchedOpt = rawOptions.find((opt: z.ZodTypeAny) => {
								const optShape = extractObjectShape(opt)
								const discProp = optShape[discKey]
								if (discProp) {
									const discVal = extractLiteralValue(discProp)
									return discVal === draft[key][discKey]
								}
								return false
							})
							if (matchedOpt) {
								draft[key] = this.toDraft(
									matchedOpt,
									draft[key],
									hints,
									fieldPath,
									root
								)
							}
						} else {
							// Check externally-tagged union:
							const matchedOpt = rawOptions.find((opt: z.ZodTypeAny) => {
								const optShape = extractObjectShape(opt)
								const optKeys = Object.keys(optShape)
								return optKeys.length === 1 && optKeys[0] in draft[key]
							})
							if (matchedOpt) {
								draft[key] = this.toDraft(
									matchedOpt,
									draft[key],
									hints,
									fieldPath,
									root
								)
							}
						}
					} else if (
						fieldUnwrapped.typeName === 'ZodObject' ||
						fieldUnwrapped.typeName === 'ZodIntersection'
					) {
						draft[key] = this.toDraft(
							fieldSchema as z.ZodTypeAny,
							draft[key],
							hints,
							fieldPath,
							root
						)
					} else if (fieldUnwrapped.typeName === 'ZodRecord') {
						if (
							!draft[key] ||
							typeof draft[key] !== 'object' ||
							Array.isArray(draft[key])
						) {
							draft[key] = {}
						}
					}
				}
			}
		}

		if (unwrapped.typeName === 'ZodUnion' || unwrapped.typeName === 'ZodDiscriminatedUnion') {
			const rawOptions = flattenUnionOptions(unwrapped.innerType)
			if (rawOptions.length > 0) {
				return this.toDraft(rawOptions[0] as z.ZodTypeAny, entity, hints, currentPath, root)
			}
		}

		return draft
	}

	/**
	 * Recursively sanitizes draft data according to schema
	 */
	static deepSanitize(
		val: any,
		schema: z.ZodTypeAny,
		currentPath: string[] = [],
		hints: Record<string, UiHint> = {},
		rootDraft?: any
	): any {
		const root = rootDraft || val
		const pathKey = pathToString(currentPath)
		let hint = hints[pathKey]
		if (!hint) {
			const normalizedKey = currentPath.filter((p) => isNaN(Number(p))).join('.')
			hint = hints[normalizedKey]
		}
		if (currentPath.length > 2 && root) {
			const resolved = resolveBranchAliasPath(root, currentPath)
			if (resolved) {
				const branchHint =
					hints[pathToString(resolved)] ||
					hints[resolved.filter((p) => isNaN(Number(p))).join('.')]
				if (branchHint) {
					hint = { ...branchHint, ...hint }
				}
			}
		}
		hint = hint || {}
		const lastSeg = currentPath[currentPath.length - 1]
		const isLabelsField =
			lastSeg === 'labels' || hint.widget === 'label' || hint.emptyPolicy === 'empty-object'

		// Special handling for 'labels' field or widget 'label' or emptyPolicy 'empty-object':
		// Always ensure a JSON object is produced (not undefined, null, or empty string)
		// as backend API (Axum/Serde/Postgres JSONB) requires this field to exist and be valid.
		if (isLabelsField) {
			if (typeof val === 'string') {
				try {
					const parsed = JSON.parse(val || '{}')
					return typeof parsed === 'object' && parsed !== null ? parsed : {}
				} catch {
					return {}
				}
			}
			if (typeof val === 'object' && val !== null) {
				return val
			}
			return {}
		}

		if (val === null || val === undefined) {
			const unwrappedVal = unwrapZodType(schema)
			if (
				val === undefined &&
				unwrappedVal.typeName === 'ZodBoolean' &&
				!unwrappedVal.isOptional &&
				!unwrappedVal.isNullable
			) {
				return unwrappedVal.hasDefault ? unwrappedVal.defaultValue : false
			}
			return val
		}

		const unwrapped = unwrapZodType(schema)

		// 1. Sanitize empty strings
		if (val === '') {
			if (hint.emptyPolicy === 'null') return null
			if (hint.emptyPolicy === 'empty-string') return ''
			return undefined
		}

		// 2. Numeric coercion
		if (unwrapped.typeName === 'ZodNumber' && typeof val === 'string' && val.trim() !== '') {
			const num = Number(val)
			return isNaN(num) ? val : num
		}

		// 3. Recurse into Objects & Intersections
		if (
			(unwrapped.typeName === 'ZodObject' || unwrapped.typeName === 'ZodIntersection') &&
			typeof val === 'object' &&
			val !== null &&
			!Array.isArray(val)
		) {
			const shape = extractObjectShape(unwrapped.innerType)
			const cleaned: Record<string, any> = {}

			for (const [key, propSchema] of Object.entries(shape)) {
				const propUnwrapped = unwrapZodType(propSchema as z.ZodTypeAny)
				if (key in val && val[key] !== undefined) {
					cleaned[key] = this.deepSanitize(
						val[key],
						propSchema as z.ZodTypeAny,
						[...currentPath, key],
						hints,
						root
					)
				} else if (propUnwrapped.hasDefault) {
					cleaned[key] = propUnwrapped.defaultValue
				} else if (propUnwrapped.typeName === 'ZodArray') {
					cleaned[key] = []
				} else if (
					propUnwrapped.typeName === 'ZodBoolean' &&
					!propUnwrapped.isOptional &&
					!propUnwrapped.isNullable
				) {
					cleaned[key] = false
				}
			}

			// Cleanings using emptyPolicy: 'null' when configured on the field
			if (hint?.emptyPolicy === 'null') {
				const isAllEmpty = Object.values(cleaned).every((v) => {
					if (v === null || v === undefined || v === '') return true
					if (Array.isArray(v) && v.length === 0) return true
					if (typeof v === 'boolean' && v === false) return true
					if (typeof v === 'object' && Object.keys(v).length === 0) return true
					return false
				})
				if (isAllEmpty) {
					return null
				}
			}

			// If this object is optional and initial input was empty -> return undefined
			if (unwrapped.isOptional && Object.keys(val).length === 0) {
				return undefined
			}

			return cleaned
		}

		// 4. Recurse into Discriminated Unions
		if (
			(unwrapped.typeName === 'ZodUnion' || unwrapped.typeName === 'ZodDiscriminatedUnion') &&
			typeof val === 'object' &&
			val !== null &&
			!Array.isArray(val)
		) {
			const rawOptions = flattenUnionOptions(unwrapped.innerType)
			const hasNull = rawOptions.some(
				(opt) =>
					unwrapZodType(opt).typeName === 'ZodNull' ||
					unwrapZodType(opt).typeName === 'ZodUndefined'
			)
			if (
				Object.keys(val).length === 0 &&
				(hasNull || unwrapped.isNullable || unwrapped.isOptional)
			) {
				return null
			}

			// Find option matching discriminator value in val
			const discKey =
				'type' in val
					? 'type'
					: 'module' in val
						? 'module'
						: 'kind' in val
							? 'kind'
							: 'method' in val
								? 'method'
								: 'strategy' in val
									? 'strategy'
									: undefined
			if (discKey && val[discKey]) {
				const matchingOpt = rawOptions.find((opt: z.ZodTypeAny) => {
					const optShape = extractObjectShape(opt)
					const discProp = optShape[discKey]
					if (discProp) {
						const discVal = extractLiteralValue(discProp)
						return discVal === val[discKey]
					}
					return false
				})

				if (matchingOpt) {
					return this.deepSanitize(val, matchingOpt, currentPath, hints, root)
				}
			} else {
				// Check externally-tagged union or single non-null option (like nullable wrapper)
				const nonNullOptions = rawOptions.filter(
					(opt) =>
						unwrapZodType(opt).typeName !== 'ZodNull' &&
						unwrapZodType(opt).typeName !== 'ZodUndefined'
				)
				if (nonNullOptions.length === 1) {
					const sanitizedInner = this.deepSanitize(
						val,
						nonNullOptions[0],
						currentPath,
						hints,
						root
					)
					if (sanitizedInner === null || sanitizedInner === undefined) {
						return hasNull || unwrapped.isNullable ? null : undefined
					}
					return sanitizedInner
				}

				const matchingOpt = rawOptions.find((opt: z.ZodTypeAny) => {
					const optShape = extractObjectShape(opt)
					const optKeys = Object.keys(optShape)
					return optKeys.length === 1 && optKeys[0] in val
				})
				if (matchingOpt) {
					return this.deepSanitize(val, matchingOpt, currentPath, hints, root)
				}
			}
		}

		// 5. Recurse into Arrays
		if (unwrapped.typeName === 'ZodArray' && Array.isArray(val)) {
			const elemSchema = (unwrapped.innerType as z.ZodArray<any>)._def.type
			return val
				.map((item, idx) =>
					this.deepSanitize(item, elemSchema, [...currentPath, String(idx)], hints, root)
				)
				.filter((item) => item !== undefined && item !== '')
		}

		// 6. Recurse into Records
		if (
			unwrapped.typeName === 'ZodRecord' &&
			typeof val === 'object' &&
			val !== null &&
			!Array.isArray(val)
		) {
			const valueSchema =
				(unwrapped.innerType as any)._def?.valueType ||
				(unwrapped.innerType as any).valueSchema
			const cleaned: Record<string, any> = {}
			for (const [k, v] of Object.entries(val)) {
				const trimmedKey = String(k).trim()
				if (trimmedKey !== '') {
					cleaned[trimmedKey] = valueSchema
						? this.deepSanitize(
								v,
								valueSchema,
								[...currentPath, trimmedKey],
								hints,
								root
							)
						: v
				}
			}
			return cleaned
		}

		return val
	}
}

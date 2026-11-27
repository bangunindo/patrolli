// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Utility functions for nested object path manipulation and form label formatting
 */

import { toRaw } from 'vue'
import type { UiHint } from '@@/shared/types/dynamic-form.type'

/**
 * Retrieves a value from a nested object based on a path array
 */
export function getValueByPath(obj: Record<string, any> | undefined | null, path: string[]): any {
	if (!obj || !path || path.length === 0) return undefined
	let curr = obj
	for (const seg of path) {
		if (curr === null || curr === undefined) return undefined
		curr = curr[seg]
	}
	return curr
}

/**
 * Sets a value on a nested object based on a path array (creates intermediate objects if needed)
 */
export function setValueByPath(obj: Record<string, any>, path: string[], value: any): void {
	if (!obj || !path || path.length === 0) return
	let curr = obj
	for (let i = 0; i < path.length - 1; i++) {
		const seg = path[i]
		if (!(seg in curr) || typeof curr[seg] !== 'object' || curr[seg] === null) {
			curr[seg] = {}
		}
		curr = curr[seg]
	}
	const lastKey = path[path.length - 1]
	curr[lastKey] = value
}

/**
 * Deletes a property on a nested object based on a path array
 */
export function deleteValueByPath(obj: Record<string, any>, path: string[]): void {
	if (!obj || !path || path.length === 0) return
	let curr = obj
	for (let i = 0; i < path.length - 1; i++) {
		const seg = path[i]
		if (!(seg in curr) || typeof curr[seg] !== 'object' || curr[seg] === null) {
			return
		}
		curr = curr[seg]
	}
	const lastKey = path[path.length - 1]
	delete curr[lastKey]
}

/**
 * Converts a path array into a dot-notation string (e.g. ['config', 'url'] -> 'config.url')
 */
export function pathToString(path: string[]): string {
	return path.join('.')
}

/**
 * Converts a dot-notation string into a path array
 */
export function stringToPath(pathStr: string): string[] {
	if (!pathStr) return []
	return pathStr.split('.')
}

/**
 * Converts snake_case, kebab-case, or camelCase to clean Title Case for form labels.
 * Example: 'max_bulk_repetitions' -> 'Max Bulk Repetitions', 'host_id' -> 'Host ID', 'admStatus' -> 'Adm Status'
 */
export function formatFieldLabel(name: string): string {
	if (!name) return ''

	// Common acronyms mapping
	const acronymMap: Record<string, string> = {
		id: 'ID',
		ip: 'IP',
		url: 'URL',
		uri: 'URI',
		http: 'HTTP',
		snmp: 'SNMP',
		oid: 'OID',
		tls: 'TLS',
		ssl: 'SSL',
		sasl: 'SASL',
		usm: 'USM',
		uuid: 'UUID',
		ms: 'MS'
	}

	return name
		.replace(/[-_]/g, ' ')
		.replace(/([a-z])([A-Z])/g, '$1 $2')
		.split(' ')
		.filter((word) => word.length > 0)
		.map((word) => {
			const lower = word.toLowerCase()
			if (acronymMap[lower]) {
				return acronymMap[lower]
			}
			return word.charAt(0).toUpperCase() + word.slice(1).toLowerCase()
		})
		.join(' ')
}

/**
 * Extracts the active discriminator value (module or type) from root form data.
 */
export function getActiveDiscriminatorType(rootData: any): string | undefined {
	return rootData?.config?.module || rootData?.config?.type || rootData?.module || rootData?.type
}

function resolveBranchHint(
	rawHints: Record<string, UiHint>,
	section: string,
	branchType: string,
	parts: string[]
): UiHint | undefined {
	if (parts.length === 2 && parts[1] === 'config') {
		return rawHints[`${section}.${branchType}.config`]
	}
	if (parts.length > 1 && parts[1] !== branchType) {
		const rest = parts[1] === 'config' ? parts.slice(2) : parts.slice(1)
		if (rest.length > 0) {
			const branchAlias = [section, branchType, ...rest].join('.')
			if (rawHints[branchAlias]) return rawHints[branchAlias]
			const branchWithConfig = [section, branchType, 'config', ...rest].join('.')
			if (rawHints[branchWithConfig]) return rawHints[branchWithConfig]
		}
	}
	return undefined
}

/**
 * Resolves UiHint for a given path key using branch-specific union aliases, exact match,
 * and numeric array-index normalization without leaf-name fallback.
 */
export function resolveHintForPath(
	pathKey: string,
	hints: Record<string, UiHint> = {},
	rootData?: any
): UiHint | undefined {
	if (!pathKey) return undefined
	const rawHints = toRaw(hints)
	const cleanKey = pathKey.replace(/^\.+|\.+$/g, '')
	if (!cleanKey) return undefined

	const rawParts = cleanKey.split('.')
	const normalizedParts = rawParts.filter((seg) => !/^\d+$/.test(seg))
	const partVariants =
		normalizedParts.length !== rawParts.length ? [rawParts, normalizedParts] : [rawParts]

	const activeType = getActiveDiscriminatorType(rootData)

	for (const parts of partVariants) {
		if (parts[0] === 'config' && activeType) {
			const branchHint = resolveBranchHint(rawHints, 'config', activeType, parts)
			if (branchHint) return branchHint
		} else if (parts.length > 1) {
			const rootSection = rootData?.[parts[0]]
			const sectionType = rootSection?.type || rootSection?.module
			if (sectionType) {
				const branchHint = resolveBranchHint(rawHints, parts[0], sectionType, parts)
				if (branchHint) return branchHint
			}
		}

		const candidateKey = parts.join('.')
		if (rawHints[candidateKey]) return rawHints[candidateKey]

		if (parts[0] === 'config' && parts.length > 1 && parts[1] !== 'config') {
			const configAlias = ['config', 'config', ...parts.slice(1)].join('.')
			if (rawHints[configAlias]) return rawHints[configAlias]
		}
	}

	return undefined
}

/**
 * Safely clones a value (unwrapping Vue reactive proxies) using structuredClone with JSON fallback.
 */
export function safeClone<T>(val: T): T {
	if (val === undefined || val === null || typeof val !== 'object') return val
	const raw = toRaw(val)
	try {
		return structuredClone(raw)
	} catch {
		return JSON.parse(JSON.stringify(raw))
	}
}

/**
 * Evaluates whether a field is visible given its resolved UiHint and current root form values.
 */
export function isFieldVisible(
	hint: UiHint | undefined,
	rootData: Record<string, any> = {}
): boolean {
	if (hint?.hidden) return false
	const vw = hint?.visibleWhen
	if (typeof vw === 'function') {
		try {
			return Boolean(vw(rootData))
		} catch {
			return true
		}
	}
	return true
}

// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Utility functions for nested object path manipulation and form label formatting
 */

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

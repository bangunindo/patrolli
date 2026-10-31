// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Format Validator Registry for Frontend Dynamic Form Engine
 * Provides a modular, independent, and pluggable format validation system.
 */

export interface ValidationContext {
	path: string[]
	pathKey: string
	label: string
	draft?: Record<string, any>
}

export interface FormatValidator {
	name: string
	test: (value: any, context: ValidationContext) => boolean
	message: string | ((context: ValidationContext) => string)
}

export const formatFieldLabel = (label: string): string => {
	if (!label) return 'Field'
	return label.toLowerCase().endsWith('field') ? label : `${label} field`
}

const UUID_REGEX = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i

/* eslint-disable no-control-regex */
const EMAIL_REGEX =
	/^(?:[a-zA-Z0-9!#$%&'*+/=?^_`{|}~-]+(?:\.[a-zA-Z0-9!#$%&'*+/=?^_`{|}~-]+)*|"(?:[\x01-\x08\x0b\x0c\x0e-\x1f\x21\x23-\x5b\x5d-\x7f]|\\[\x01-\x09\x0b\x0c\x0e-\x7f])*")@(?:(?:[a-zA-Z0-9](?:[a-zA-Z0-9-]*[a-zA-Z0-9])?\.)+[a-zA-Z0-9](?:[a-zA-Z0-9-]*[a-zA-Z0-9])?|\[(?:(?:(2(5[0-5]|[0-4][0-9])|1[0-9][0-9]|[1-9]?[0-9]))\.){3}(?:(2(5[0-5]|[0-4][0-9])|1[0-9][0-9]|[1-9]?[0-9])|[a-zA-Z0-9-]*[a-zA-Z0-9]:(?:[\x01-\x08\x0b\x0c\x0e-\x1f\x21-\x5a\x53-\x7f]|\\[\x01-\x09\x0b\x0c\x0e-\x7f])+)\])$/
/* eslint-enable no-control-regex */

const IPV4_REGEX =
	/^(?:(?:25[0-5]|2[0-4][0-9]|1[0-9]{2}|[1-9]?[0-9])\.){3}(?:25[0-5]|2[0-4][0-9]|1[0-9]{2}|[1-9]?[0-9])$/

const IPV6_REGEX =
	/^(([0-9a-fA-F]{1,4}:){7,7}[0-9a-fA-F]{1,4}|([0-9a-fA-F]{1,4}:){1,7}:|([0-9a-fA-F]{1,4}:){1,6}:[0-9a-fA-F]{1,4}|([0-9a-fA-F]{1,4}:){1,5}(:[0-9a-fA-F]{1,4}){1,2}|([0-9a-fA-F]{1,4}:){1,4}(:[0-9a-fA-F]{1,4}){1,3}|([0-9a-fA-F]{1,4}:){1,3}(:[0-9a-fA-F]{1,4}){1,4}|([0-9a-fA-F]{1,4}:){1,2}(:[0-9a-fA-F]{1,4}){1,5}|[0-9a-fA-F]{1,4}:((:[0-9a-fA-F]{1,4}){1,6})|:((:[0-9a-fA-F]{1,4}){1,7}|:)|fe80:(:[0-9a-fA-F]{0,4}){0,4}%[0-9a-zA-Z]{1,}|::(ffff(:0{1,4}){0,1}:){0,1}((25[0-5]|(2[0-4]|1{0,1}[0-9]){0,1}[0-9])\.){3,3}(25[0-5]|(2[0-4]|1{0,1}[0-9]){0,1}[0-9])|([0-9a-fA-F]{1,4}:){1,4}:((25[0-5]|(2[0-4]|1{0,1}[0-9]){0,1}[0-9])\.){3,3}(25[0-5]|(2[0-4]|1{0,1}[0-9]){0,1}[0-9]))$/

const IPV4_CIDR_REGEX =
	/^(?:(?:25[0-5]|2[0-4][0-9]|1[0-9]{2}|[1-9]?[0-9])\.){3}(?:25[0-5]|2[0-4][0-9]|1[0-9]{2}|[1-9]?[0-9])\/(?:[0-9]|[1-2][0-9]|3[0-2])$/

const HOSTNAME_REGEX =
	/^([a-zA-Z0-9]|[a-zA-Z0-9][a-zA-Z0-9-]{0,61}[a-zA-Z0-9])(\.([a-zA-Z0-9]|[a-zA-Z0-9][a-zA-Z0-9-]{0,61}[a-zA-Z0-9]))*$/

const CRON_PRESETS = [
	'@yearly',
	'@annually',
	'@monthly',
	'@weekly',
	'@daily',
	'@midnight',
	'@hourly'
]

// Validator storage registry
const formatRegistry: Map<string, FormatValidator> = new Map()

/**
 * Register built-in standard format validators
 */
const builtInValidators: FormatValidator[] = [
	{
		name: 'required',
		test: (val) => {
			if (val === null || val === undefined) return false
			if (typeof val === 'string' || typeof val === 'number') {
				return `${val}`.trim().length > 0
			}
			if (Array.isArray(val)) {
				return val.length > 0
			}
			if (typeof val === 'object') {
				return Object.keys(val).length > 0
			}
			return Boolean(val)
		},
		message: (ctx) => `${formatFieldLabel(ctx.label)} is required`
	},
	{
		name: 'array',
		test: (val) => Array.isArray(val) && val.length > 0,
		message: (ctx) => `${formatFieldLabel(ctx.label)} can't be empty`
	},
	{
		name: 'uuid',
		test: (val) => {
			if (!val || typeof val !== 'string') return true // Optional check is handled by the required rule
			return UUID_REGEX.test(val.trim())
		},
		message: (ctx) => `${formatFieldLabel(ctx.label)} must be a valid UUID`
	},
	{
		name: 'email',
		test: (val) => {
			if (!val || typeof val !== 'string') return true
			return EMAIL_REGEX.test(val.trim())
		},
		message: (ctx) => `${formatFieldLabel(ctx.label)} must be a valid email address`
	},
	{
		name: 'ipv4',
		test: (val) => {
			if (!val || typeof val !== 'string') return true
			return IPV4_REGEX.test(val.trim())
		},
		message: (ctx) =>
			`${formatFieldLabel(ctx.label)} must be a valid IPv4 address (e.g. 192.168.1.1)`
	},
	{
		name: 'ipv6',
		test: (val) => {
			if (!val || typeof val !== 'string') return true
			return IPV6_REGEX.test(val.trim())
		},
		message: (ctx) => `${formatFieldLabel(ctx.label)} must be a valid IPv6 address`
	},
	{
		name: 'ip',
		test: (val) => {
			if (!val || typeof val !== 'string') return true
			const trimmed = val.trim()
			return IPV4_REGEX.test(trimmed) || IPV6_REGEX.test(trimmed)
		},
		message: (ctx) => `${formatFieldLabel(ctx.label)} must be a valid IP address`
	},
	{
		name: 'ip-or-cidr',
		test: (val) => {
			if (!val || typeof val !== 'string') return true
			const trimmed = val.trim()
			return (
				IPV4_REGEX.test(trimmed) ||
				IPV6_REGEX.test(trimmed) ||
				IPV4_CIDR_REGEX.test(trimmed)
			)
		},
		message: (ctx) =>
			`${formatFieldLabel(ctx.label)} must be a valid IP address or CIDR subnet (e.g. 10.0.0.0/24)`
	},
	{
		name: 'hostname',
		test: (val) => {
			if (!val || typeof val !== 'string') return true
			const trimmed = val.trim()
			return HOSTNAME_REGEX.test(trimmed) && trimmed.length <= 253
		},
		message: (ctx) => `${formatFieldLabel(ctx.label)} must be a valid hostname or domain`
	},
	{
		name: 'port',
		test: (val) => {
			if (val === undefined || val === null || val === '') return true
			const num = Number(val)
			return Number.isInteger(num) && num >= 1 && num <= 65535
		},
		message: (ctx) => `${formatFieldLabel(ctx.label)} must be a valid port number (1-65535)`
	},
	{
		name: 'url',
		test: (val) => {
			if (!val || typeof val !== 'string') return true
			try {
				const url = new URL(val.trim())
				return ['http:', 'https:', 'ws:', 'wss:'].includes(url.protocol)
			} catch {
				return false
			}
		},
		message: (ctx) =>
			`${formatFieldLabel(ctx.label)} must be a valid URL (http, https, ws, wss)`
	},
	{
		name: 'json',
		test: (val) => {
			if (!val || typeof val !== 'string') return true
			try {
				JSON.parse(val.trim())
				return true
			} catch {
				return false
			}
		},
		message: (ctx) => `${formatFieldLabel(ctx.label)} must be a valid JSON string`
	},
	{
		name: 'cron',
		test: (val) => {
			if (!val || typeof val !== 'string') return true
			const trimmed = val.trim()
			if (CRON_PRESETS.includes(trimmed)) return true
			// Patrolli schedule format: integer seconds (e.g. '10s', '60') or 5-part cron
			if (/^\d+s?$/i.test(trimmed)) return true
			const parts = trimmed.split(/\s+/)
			return parts.length === 5 || parts.length === 6
		},
		message: (ctx) =>
			`${formatFieldLabel(ctx.label)} must be a valid schedule or cron expression`
	}
]

// Register all built-ins into registry
for (const validator of builtInValidators) {
	formatRegistry.set(validator.name, validator)
}

/**
 * Register a new format validator into the registry
 */
export function registerFormatValidator(validator: FormatValidator): void {
	formatRegistry.set(validator.name, validator)
}

/**
 * Get validator by identifier name
 */
export function getFormatValidator(name: string): FormatValidator | undefined {
	return formatRegistry.get(name)
}

/**
 * Check if a format validator is registered
 */
export function hasFormatValidator(name: string): boolean {
	return formatRegistry.has(name)
}

/**
 * Get all registered format validators
 */
export function getAllFormatValidators(): FormatValidator[] {
	return Array.from(formatRegistry.values())
}

/**
 * Validate a value against a registered format validator.
 * Returns an error message string if invalid, or null if valid.
 */
export function validateFormat(
	formatName: string,
	value: any,
	context: ValidationContext
): string | null {
	const validator = formatRegistry.get(formatName)
	if (!validator) return null

	const isValid = validator.test(value, context)
	if (isValid) return null

	if (typeof validator.message === 'function') {
		return validator.message(context)
	}
	return validator.message
}

/**
 * Evaluates a list of rules against a given value using the validator registry.
 * Returns an array of error messages (string[]).
 */
export function validateRules(value: any, rules: string[], context: ValidationContext): string[] {
	if (!rules || rules.length === 0) return []
	const messages: string[] = []

	// 1. Execute 'required' rule if present
	if (rules.includes('required')) {
		const err = validateFormat('required', value, context)
		if (err) {
			messages.push(err)
			return messages
		}
	}

	// 2. Execute 'array' rule if present
	if (rules.includes('array')) {
		const err = validateFormat('array', value, context)
		if (err) {
			messages.push(err)
			return messages
		}
	}

	// 3. Execute other format rules
	for (const rule of rules) {
		if (rule === 'required' || rule === 'array') continue

		if (hasFormatValidator(rule)) {
			if (Array.isArray(value)) {
				let firstInvalidErr: string | null = null
				for (const item of value) {
					const err = validateFormat(rule, item, context)
					if (err) {
						firstInvalidErr = err
						break
					}
				}
				if (firstInvalidErr) {
					if (rule === 'uuid') {
						messages.push(`${formatFieldLabel(context.label)} must contain valid UUIDs`)
					} else {
						messages.push(firstInvalidErr)
					}
				}
			} else {
				if (value === '' || value === null || value === undefined) {
					continue
				}
				const err = validateFormat(rule, value, context)
				if (err) {
					messages.push(err)
				}
			}
		}
	}

	return messages
}

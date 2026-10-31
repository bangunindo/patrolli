// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Core contract types for Frontend Dynamic Form Engine.
 * Supports automated Zod schema introspection, declarative UI hints,
 * and integration with the Widget Registry.
 */

/** Widget type supported by the registry */
export type FieldWidgetType =
	| 'text'
	| 'number'
	| 'switch'
	| 'select'
	| 'password'
	| 'array-string'
	| 'array-object'
	| 'record'
	| 'json'
	| 'discriminated-union'
	| 'attribute-filter'
	| 'custom'
	| (string & {})

/** Nested property path (e.g. ['config', 'snmp_v3', 'username']) */
export type FieldPath = string[]

/** Empty string handling policy during sanitization */
export type EmptyPolicy = 'undefined' | 'null' | 'empty-string' | 'empty-object'

/** Visual options injected via UI Hints */
export interface UiHint {
	/** Override widget type in registry */
	widget?: FieldWidgetType

	/** Custom display label (default: Title Case of property name) */
	label?: string

	/** Placeholder text on input element */
	placeholder?: string

	/** Help text or description beneath input */
	description?: string

	/** Tooltip or help text */
	helpText?: string

	/** Mark value as sensitive credential (renders password widget) */
	sensitive?: boolean

	/** Display render order (lower values render earlier) */
	order?: number

	/** Group field into a Card/FieldGroup with this title */
	group?: string

	/** Group visual variant ('card' with border or 'section' borderless) */
	groupVariant?: 'card' | 'section'

	/** Conditional visibility rule (expression string or callback function) */
	visibleWhen?: string | ((draft: Record<string, any>) => boolean)

	/** Static dropdown options or asynchronous fetch function */
	options?:
		| Array<{ label: string; value: any }>
		| (() => Promise<Array<{ label: string; value: any }>>)

	/** Specific options for keys in record widgets */
	keyOptions?: Array<{ label: string; value: any }>

	/** Custom label for keys in record widgets */
	keyLabel?: string

	/** Custom placeholder for keys in record widgets */
	keyPlaceholder?: string

	/** Custom placeholder for values in record widgets */
	valuePlaceholder?: string

	/** Policy when input is cleared ('undefined' | 'null' | 'empty-string') */
	emptyPolicy?: EmptyPolicy

	/** Lock input to prevent editing */
	disabled?: boolean

	/** Switch input to read-only mode */
	readonly?: boolean

	/** Responsive grid column span (1-12) */
	colSpan?: 1 | 2 | 3 | 4 | 6 | 12

	/** Hide field completely from rendered form */
	hidden?: boolean

	/** Hide border/title of FieldGroup on nested objects */
	hideGroup?: boolean

	/** Custom label for discriminated union variant selector */
	discriminatorLabel?: string

	/** Custom placeholder for discriminated union variant selector */
	discriminatorPlaceholder?: string

	/** Custom discriminator field (default: 'type' / auto-detect) */
	discriminatorField?: string

	/** Default discriminator value if none is selected */
	discriminatorDefault?: string

	/** Order of discriminator variant choices */
	optionsOrder?: string[]

	/** Format validator name (e.g. 'uuid', 'email', 'ip', 'ipv4', 'hostname', 'port', 'url', 'cron') */
	format?: string

	/** List of built-in validation rules or format names (e.g. ['required', 'ip']) */
	rules?: string[]

	/** Custom validator function specific to this field */
	validator?: (
		value: any,
		draft: Record<string, any>
	) =>
		| string
		| boolean
		| Record<string, string>
		| undefined
		| null
		| void
		| Promise<string | boolean | Record<string, string> | undefined | null | void>

	/** Initial default value for this field */
	defaultValue?: any

	/** Custom error message overriding default Zod or format message */
	errorMessage?: string
}

/** Metadata representation of a field resulting from Zod introspection */
export interface FieldDescriptor {
	name: string
	path: FieldPath
	pathKey: string
	label: string
	widget: FieldWidgetType
	zodType: string
	required: boolean
	isOptional: boolean
	isNullable: boolean
	defaultValue?: any
	description?: string
	placeholder?: string
	min?: number
	max?: number
	step?: number
	format?: string
	rules?: string[]
	pattern?: string
	regex?: RegExp
	options?: Array<{ label: string; value: any }>
	children?: FieldDescriptor[]
	unionDiscriminator?: string
	unionTagMode?: 'internal' | 'external'
	unionBranches?: Record<string, FieldDescriptor[]>
	hints?: UiHint
}

/** Field-level validation error message */
export interface FieldIssue {
	path: string
	message: string
}

export interface FormValidationSuccess<T> {
	success: true
	data: T
	issues?: never
}

export interface FormValidationFailure {
	success: false
	issues: Record<string, string>
	data?: never
}

export type FormValidationResult<T = any> = FormValidationSuccess<T> | FormValidationFailure

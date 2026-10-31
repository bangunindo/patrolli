// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

/** This is temporary File, it will be removed in the future */

export declare type RuleConjunction = 'AND' | 'OR'

export declare type RuleOperator =
	| 'EQUAL'
	| 'NOT_EQUAL'
	| 'GREATER_THAN'
	| 'LESS_THAN'
	| 'GREATER_THAN_OR_EQUAL'
	| 'LESS_THAN_OR_EQUAL'
	| 'CONTAINS'
	| 'NOT_CONTAINS'
	| 'STARTS_WITH'
	| 'ENDS_WITH'
	| 'IS_NULL'
	| 'IS_NOT_NULL'

export declare interface RuleConfig {
	conjunction?: RuleConjunction
	field: string
	operator: RuleOperator
	value: string
}

export declare interface IRules {
	id: string
	title: string
	description?: string
	config?: RuleConfig[] | null
}

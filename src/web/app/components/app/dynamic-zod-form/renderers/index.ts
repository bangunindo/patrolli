// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import {
	isAnyOfControl,
	isBooleanControl,
	isEnumControl,
	isGroup,
	isIntegerControl,
	isLayout,
	isNumberControl,
	isObjectControl,
	isOneOfControl,
	isStringControl,
	optionIs,
	or,
	rankWith,
	schemaMatches,
	schemaTypeIs
} from '@jsonforms/core'
import type { JsonFormsRendererRegistryEntry } from '@jsonforms/core'

import { markRaw } from 'vue'

import VerticalLayout from './VerticalLayout.vue'
import GroupLayout from './GroupLayout.vue'
import TextControl from './TextControl.vue'
import PasswordControl from './PasswordControl.vue'
import SwitchControl from './SwitchControl.vue'
import NumberControl from './NumberControl.vue'
import BooleanControl from './BooleanControl.vue'
import SelectControl from './SelectControl.vue'
import UnionControl from './UnionControl.vue'
import LabelControl from './LabelControl.vue'
import ObjectControl from './ObjectControl.vue'
import ArrayControl from './ArrayControl.vue'
import AttributeFilterControl from './AttributeFilterControl.vue'
import RecordControl from './RecordControl.vue'
import LogAttrsControl from './LogAttrsControl.vue'
import CodeControl from './CodeControl.vue'

// Custom tester for LogRow attrs (tuple array [[Key, Value], ...])
const isLogAttrsControl = or(
	optionIs('widget', 'log-attrs'),
	optionIs('widget', 'attrs'),
	schemaMatches((schema) => {
		if (!schema || schema.type !== 'array') return false
		const tupleItems = Array.isArray(schema.items) ? schema.items : (schema as any).prefixItems
		return Array.isArray(tupleItems) && tupleItems.length === 2
	})
)

// Custom tester for Record fields (schema has additionalProperties without explicit properties)
const isRecordControl = schemaMatches((schema) => {
	if (!schema || schema.type !== 'object') return false
	if (
		schema.additionalProperties &&
		(!schema.properties || Object.keys(schema.properties).length === 0)
	) {
		return true
	}
	return false
})

// Custom tester for array controls
const isArrayFieldControl = or(
	schemaTypeIs('array'),
	schemaMatches((schema) => {
		if (!schema) return false
		return schema.type === 'array' || Array.isArray(schema.items) || !!schema.items
	})
)

// Custom tester for enum / select controls (including nullable enums like anyOf: [{type: 'null'}, {type: 'string', enum: ...}])
const isSelectControl = or(
	isEnumControl,
	optionIs('widget', 'select'),
	schemaMatches((schema) => {
		if (!schema) return false
		if (schema.enum) return true
		if (Array.isArray(schema.anyOf)) {
			return schema.anyOf.some((sub: any) => sub && sub.enum)
		}
		if (Array.isArray(schema.oneOf)) {
			return schema.oneOf.every(
				(sub: any) => sub && (sub.const !== undefined || sub.enum || sub.type === 'null')
			)
		}
		return false
	})
)

export const zodFormRenderers: JsonFormsRendererRegistryEntry[] = [
	// Layout Renderers
	{ tester: rankWith(1, isLayout), renderer: markRaw(VerticalLayout) },
	{ tester: rankWith(2, isGroup), renderer: markRaw(GroupLayout) },

	// Specialized Widgets
	{
		tester: rankWith(7, isLogAttrsControl),
		renderer: markRaw(LogAttrsControl)
	},
	{
		tester: rankWith(6, optionIs('widget', 'attribute-filter')),
		renderer: markRaw(AttributeFilterControl)
	},
	{
		tester: rankWith(6, or(optionIs('widget', 'code'), optionIs('widget', 'opl'))),
		renderer: markRaw(CodeControl)
	},
	{
		tester: rankWith(6, or(optionIs('widget', 'record'), isRecordControl)),
		renderer: markRaw(RecordControl)
	},

	// Control Renderers
	{
		tester: rankWith(
			5,
			or(
				optionIs('widget', 'text'),
				optionIs('widget', 'textarea'),
				optionIs('widget', 'date')
			)
		),
		renderer: markRaw(TextControl)
	},
	{
		tester: rankWith(5, isSelectControl),
		renderer: markRaw(SelectControl)
	},
	{
		tester: rankWith(5, isObjectControl),
		renderer: markRaw(ObjectControl)
	},
	{
		tester: rankWith(5, isArrayFieldControl),
		renderer: markRaw(ArrayControl)
	},
	{
		tester: rankWith(5, or(optionIs('sensitive', true), optionIs('widget', 'password'))),
		renderer: markRaw(PasswordControl)
	},
	{
		tester: rankWith(5, optionIs('widget', 'switch')),
		renderer: markRaw(SwitchControl)
	},
	{
		tester: rankWith(
			5,
			or(
				optionIs('widget', 'label'),
				optionIs('emptyPolicy', 'empty-object'),
				schemaMatches(
					(schema) =>
						schema && typeof schema === 'object' && Object.keys(schema).length === 0
				)
			)
		),
		renderer: markRaw(LabelControl)
	},
	{
		tester: rankWith(4, or(isAnyOfControl, isOneOfControl)),
		renderer: markRaw(UnionControl)
	},
	{
		tester: rankWith(2, or(isNumberControl, isIntegerControl)),
		renderer: markRaw(NumberControl)
	},
	{
		tester: rankWith(2, isBooleanControl),
		renderer: markRaw(BooleanControl)
	},
	{
		tester: rankWith(1, isStringControl),
		renderer: markRaw(TextControl)
	}
]

export const btjRenderers = zodFormRenderers

// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { UiHint } from '@@/shared/types/dynamic-form.type'

const tableOptions = [
	{ label: 'RECORD', value: 'record' },
	{ label: 'SCOPE', value: 'scope' },
	{ label: 'RESOURCE', value: 'resource' }
]

export const processorHints: Record<string, UiHint> = {
	name: { order: 1, placeholder: 'Name of processor' },
	adm_status: { order: 2, label: 'Status', widget: 'switch' },
	input_items: { order: 3, label: 'Input Items' },
	input_processors: { order: 4, label: 'Input Processors' },
	input_pipelines: { order: 4, label: 'Input Processors' },
	config: {
		order: 5,
		label: 'Config',
		discriminatorLabel: 'Processor Action',
		discriminatorPlaceholder: 'Select processor action...'
	},

	// Hide double nested borders on union branches
	'config.config': { hideGroup: true },
	'config.config.keys': { keyOptions: tableOptions },
	'config.config.rename_map': { keyOptions: tableOptions },
	'config.config.kv_map': { keyOptions: tableOptions },
	'config.config.overwrite': { label: 'Overwrite Existing Data', defaultValue: false },

	// 1. Attribute Filter
	'config.attribute_filter.config': {
		widget: 'attribute-filter',
		hideGroup: true,
		label: 'Attribute Filter'
	},

	// 2. Drop Attributes (Record<Table, string[]>)
	'config.drop_attributes.config': { hideGroup: true },
	'config.drop_attributes.config.keys': {
		label: 'Drop Attributes',
		keyLabel: 'Table',
		keyOptions: tableOptions,
		description: 'Select attribute table and list keys to drop.'
	},
	'config.drop_attributes.config.keys.value': {
		label: 'Attribute Keys',
		placeholder: 'Attribute key (e.g.: http.status_code)'
	},
	'config.keys': {
		label: 'Drop Attributes',
		keyLabel: 'Table',
		keyOptions: tableOptions
	},

	// 3. Rename Attributes (Record<Table, Record<OldName, NewName>>)
	'config.rename_attributes.config': { hideGroup: true },
	'config.rename_attributes.config.rename_map': {
		label: 'Rename Attributes Mapping',
		keyLabel: 'Table',
		keyOptions: tableOptions,
		description: 'Map old attribute names to new attribute names by table.'
	},
	'config.rename_attributes.config.rename_map.value': {
		label: 'Attributes',
		keyLabel: 'Old Name',
		keyPlaceholder: 'Old attribute name',
		valuePlaceholder: 'New attribute name'
	},
	'config.rename_map': {
		label: 'Rename Attributes Mapping',
		keyLabel: 'Table',
		keyOptions: tableOptions
	},

	// 4. Add Attributes (Record<Table, Record<AttrName, TransformValue>>)
	'config.add_attributes.config': { hideGroup: true },
	'config.add_attributes.config.overwrite': {
		label: 'Overwrite Existing',
		order: 1
	},
	'config.add_attributes.config.kv_map': {
		label: 'Attributes to Add',
		keyLabel: 'Table',
		keyOptions: tableOptions,
		order: 2,
		description: 'Add new attributes to the selected telemetry table.'
	},
	'config.add_attributes.config.kv_map.value': {
		label: 'Attributes',
		keyLabel: 'Attribute Name',
		keyPlaceholder: 'e.g. service.env'
	},
	'config.add_attributes.config.kv_map.value.value': {
		hideGroup: true,
		discriminatorLabel: 'Value Type'
	},
	'config.kv_map': {
		label: 'Attributes to Add',
		keyLabel: 'Table',
		keyOptions: tableOptions
	},

	// 5. Attribute Lookup
	'config.attribute_lookup.config': { hideGroup: true },
	'config.attribute_lookup.config.when': {
		widget: 'attribute-filter',
		label: 'Condition (When)'
	},

	// 6. Transform (OPL - OpenTelemetry Processing Language)
	'config.transform.config': { hideGroup: true },
	'config.transform.config.opl': {
		widget: 'opl',
		label: 'OPL Program',
		placeholder:
			'// e.g. logs | where severity_number >= 17 | set attributes["critical"] = true'
	},
	'config.config.opl': {
		widget: 'opl',
		label: 'OPL Program',
		placeholder:
			'// e.g. logs | where severity_number >= 17 | set attributes["critical"] = true'
	}
}

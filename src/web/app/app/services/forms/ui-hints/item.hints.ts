// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { UiHint } from '@@/shared/types/dynamic-form.type'
import { SeverityNumberSchema } from '@@/shared/generated/patrolliAPI.schemas.type'

export const itemHints: Record<string, UiHint> = {
	name: { order: 1, placeholder: 'Name of item' },
	flags: { order: 2, label: 'Flags', placeholder: '0' },
	schedule: { order: 3, label: 'Schedule (Seconds)', placeholder: '30' },
	timeout_ms: { order: 4, label: 'Timeout (ms)', placeholder: '1000' },
	endpoint_id: { order: 5, label: 'Endpoint' },
	listener_id: { order: 6, label: 'Listener' },
	adm_status: { order: 7, label: 'Status', widget: 'switch' },
	labels: { order: 8, label: 'Labels', widget: 'label', emptyPolicy: 'empty-object' },
	config: {
		order: 9,
		discriminatorLabel: 'Collector Module',
		discriminatorPlaceholder: 'Select collector module...',
		discriminatorDefault: 'snmp',
		optionsOrder: ['snmp', 'snmp_walk', 'prometheus_scrape', 'listener', 'dummy']
	},

	// Hide double-nested config borders on union branches
	'config.config': { hideGroup: true },
	'config.snmp.config': { hideGroup: true },
	'config.snmp_walk.config': { hideGroup: true },
	'config.prometheus_scrape.config': { hideGroup: true },
	'config.listener.config': { hideGroup: true },
	'config.dummy.config': { hideGroup: true },

	// Collector Module: SNMP
	'config.config.oid': {
		label: 'OID',
		placeholder: '1.3.6.1.2.1.1.1.0'
	},
	'config.snmp.config.oid': {
		label: 'OID',
		placeholder: '1.3.6.1.2.1.1.1.0'
	},

	// Collector Module: SNMP Walk
	'config.snmp_walk.config.chunk_size': {
		label: 'Chunk Size',
		placeholder: '10'
	},
	'config.snmp_walk.config.disable_bulk': {
		label: 'Disable Bulk',
		widget: 'switch',
		defaultValue: false
	},
	'config.snmp_walk.config.max_results': {
		label: 'Max Results',
		placeholder: '100'
	},
	'config.snmp_walk.config.max_unknown_oids_per_row': {
		label: 'Max Unknown OIDs Per Row',
		placeholder: '0'
	},
	'config.snmp_walk.config.poll_worker_count': {
		label: 'Poll Worker Count',
		placeholder: '1'
	},
	'config.snmp_walk.config.request_retries': {
		label: 'Request Retries',
		placeholder: '3'
	},
	'config.snmp_walk.config.request_timeout': {
		label: 'Request Timeout (ms)',
		placeholder: '5000',
		defaultValue: 5000
	},
	'config.snmp_walk.config.table_format': {
		label: 'Table Format',
		widget: 'switch',
		defaultValue: false
	},
	'config.snmp_walk.config.use_mib': {
		label: 'Use MIB',
		widget: 'switch',
		defaultValue: false
	},
	'config.snmp_walk.config.oids': {
		label: 'OIDs'
	},
	'config.snmp_walk.config.oids.oid': {
		label: 'OID',
		placeholder: '1.3.6.1.2.1.1.1.0'
	},
	'config.snmp_walk.config.oids.chain_strategy': {
		label: 'Chain Strategy',
		discriminatorLabel: 'Strategy',
		discriminatorField: 'strategy',
		discriminatorDefault: 'value'
	},
	'config.snmp_walk.config.oids.chain': {
		label: 'Chain Items'
	},

	// Collector Module: Prometheus Scrape
	'config.prometheus_scrape.config.accept_protobuf': {
		label: 'Accept Protobuf',
		widget: 'switch',
		defaultValue: false
	},
	'config.prometheus_scrape.config.stream': {
		label: 'Stream',
		widget: 'switch',
		defaultValue: false
	},

	// Collector Module: Dummy
	'config.dummy.config.count': {
		label: 'Count',
		placeholder: '1'
	},
	'config.dummy.config.payload': {
		hideGroup: true
	},
	'config.dummy.config.payload.row': {
		hideGroup: true
	},
	'config.dummy.config.payload.row.observed_time': {
		label: 'Observed Time',
		widget: 'date',
		emptyPolicy: 'null'
	},
	'config.dummy.config.payload.row.severity': {
		label: 'Severity',
		widget: 'select',
		defaultValue: 'info',
		emptyPolicy: 'null',
		options: SeverityNumberSchema.options.map((s) => ({
			label: s.toUpperCase(),
			value: s
		}))
	},
	'config.dummy.config.payload.row.source_ip': {
		label: 'Source IP',
		placeholder: '127.0.0.1 (optional)',
		emptyPolicy: 'null'
	},
	'config.dummy.config.payload.row.source_port': {
		label: 'Source Port',
		widget: 'text',
		placeholder: 'Enter Source Port',
		emptyPolicy: 'null'
	},
	'config.dummy.config.payload.row.body': {
		label: 'Body',
		discriminatorLabel: 'Body Type',
		discriminatorPlaceholder: 'Select Body Type',
		discriminatorDefault: 'none',
		optionsOrder: ['none', 'String', 'SmolStr']
	},
	'config.dummy.config.payload.row.body.String': {
		label: 'Body Message',
		widget: 'textarea',
		placeholder: 'Log message body (optional)'
	},
	'config.dummy.config.payload.row.body.SmolStr': {
		label: 'Body Message',
		widget: 'text',
		placeholder: 'Short log message body (optional)'
	},
	'config.dummy.config.payload.row.attrs': {
		label: 'Attributes',
		widget: 'log-attrs',
		defaultValue: []
	},

	// Collector Module: Listener
	'config.listener.config.match_strategy': {
		label: 'Match Strategy',
		discriminatorLabel: 'Match Strategy',
		discriminatorPlaceholder: 'Select method',
		discriminatorField: 'method',
		discriminatorDefault: 'none',
		optionsOrder: ['none', 'source_ips', 'endpoint_source_ip', 'attribute_match']
	},
	'config.listener.config.match_strategy.config': { hideGroup: true },
	'config.listener.config.match_strategy.source_ips.config': {
		label: 'Source IPs',
		placeholder: '192.168.1.1'
	},
	'config.listener.config.match_strategy.attribute_match.config': {
		widget: 'attribute-filter',
		label: 'Match Filter',
		hideGroup: true
	}
}

/**
 * Specific hints for APPLIED_TEMPLATE:
 * If name, schedule, or timeout_ms are left empty, send null to inherit from parent template.
 */
export const appliedTemplateItemHints: Record<string, UiHint> = {
	name: {
		order: 1,
		emptyPolicy: 'null',
		placeholder: 'Leave blank to inherit from template'
	},
	schedule: {
		order: 3,
		label: 'Schedule (Seconds)',
		emptyPolicy: 'null',
		placeholder: 'Leave blank to inherit from template'
	},
	timeout_ms: {
		order: 4,
		label: 'Timeout (ms)',
		emptyPolicy: 'null',
		placeholder: 'Leave blank to inherit from template'
	},
	'config.type': {
		disabled: true
	}
}

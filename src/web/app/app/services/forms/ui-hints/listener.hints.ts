// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { UiHint } from '@@/shared/types/dynamic-form.type'

export const listenerHints: Record<string, UiHint> = {
	name: { order: 1, placeholder: 'Name of listener' },
	adm_status: { order: 2, label: 'Status', widget: 'switch' },
	disable_cluster_forwarding: { order: 3, label: 'Cluster Forwarding', widget: 'switch' },
	worker_count: { order: 4, label: 'Worker Count' },
	labels: { order: 5, label: 'Labels', widget: 'label', emptyPolicy: 'empty-object' },
	batch: { order: 6, label: 'Batch Configuration', hideGroup: true },
	'batch.batch_size': {
		label: 'Batch Size',
		group: 'Batch Configuration',
		defaultValue: 1000,
		placeholder: '1000'
	},
	'batch.batch_timeout': {
		label: 'Batch Timeout (ms)',
		group: 'Batch Configuration',
		defaultValue: 1000,
		placeholder: '1000'
	},
	config: {
		order: 7,
		discriminatorLabel: 'Listener Type',
		discriminatorPlaceholder: 'Select listener type...'
	},

	// Hide double nested config borders on union branches
	'config.config': { hideGroup: true },

	// Dummy Listener
	'config.dummy.messages_per_batch': {
		label: 'Messages Per Batch',
		group: 'Dummy Configuration',
		defaultValue: 100,
		placeholder: '100'
	},
	'config.config.messages_per_batch': {
		label: 'Messages Per Batch',
		group: 'Dummy Configuration',
		defaultValue: 100,
		placeholder: '100'
	},
	'config.dummy.speed': {
		label: 'Speed',
		group: 'Dummy Configuration',
		defaultValue: 5,
		placeholder: '5'
	},
	'config.config.speed': {
		label: 'Speed',
		group: 'Dummy Configuration',
		defaultValue: 5,
		placeholder: '5'
	},

	// SNMP Trap
	'config.config.batch_size': {
		label: 'Batch Size',
		group: 'Batch Configuration',
		defaultValue: 1000,
		placeholder: '1000'
	},
	'config.snmp_trap.batch_size': {
		label: 'Batch Size',
		group: 'Batch Configuration',
		defaultValue: 1000,
		placeholder: '1000'
	},
	'config.config.batch_timeout': {
		label: 'Batch Timeout (ms)',
		group: 'Batch Configuration',
		defaultValue: 1000,
		placeholder: '1000'
	},
	'config.snmp_trap.batch_timeout': {
		label: 'Batch Timeout (ms)',
		group: 'Batch Configuration',
		defaultValue: 1000,
		placeholder: '1000'
	},
	'config.config.use_mib': {
		label: 'Use MIB',
		widget: 'switch',
		defaultValue: false
	},
	'config.snmp_trap.use_mib': {
		label: 'Use MIB',
		widget: 'switch',
		defaultValue: false
	},
	'config.config.listener': {
		label: 'UDP Listener Configuration'
	},
	'config.snmp_trap.listener': {
		label: 'UDP Listener Configuration'
	},
	'config.snmp_trap.listener.address': {
		label: 'Listener Address',
		placeholder: '0.0.0.0:162',
		defaultValue: '0.0.0.0:162'
	},
	'config.config.listener.buffer_cap': {
		label: 'Buffer Capacity',
		placeholder: '100'
	},
	'config.snmp_trap.listener.buffer_cap': {
		label: 'Buffer Capacity',
		placeholder: '100'
	},
	'config.config.listener.max_packet_size': {
		label: 'Max Packet Size (bytes)',
		placeholder: '4096'
	},
	'config.snmp_trap.listener.max_packet_size': {
		label: 'Max Packet Size (bytes)',
		placeholder: '4096'
	},
	'config.config.listener.worker_count': {
		label: 'Worker Count',
		placeholder: '1'
	},
	'config.snmp_trap.listener.worker_count': {
		label: 'Worker Count',
		placeholder: '1'
	},
	'config.config.listener.ip_whitelist': {
		label: 'IP Whitelist',
		widget: 'array-string',
		placeholder: 'e.g. 192.168.1.1'
	},
	'config.snmp_trap.listener.ip_whitelist': {
		label: 'IP Whitelist',
		widget: 'array-string',
		placeholder: 'e.g. 192.168.1.1'
	},
	'config.config.listener.ip_blacklist': {
		label: 'IP Blacklist',
		widget: 'array-string',
		placeholder: 'e.g. 10.0.0.1'
	},
	'config.snmp_trap.listener.ip_blacklist': {
		label: 'IP Blacklist',
		widget: 'array-string',
		placeholder: 'e.g. 10.0.0.1'
	},
	'config.config.v1': {
		label: 'SNMP v1 Trap',
		emptyPolicy: 'null'
	},
	'config.snmp_trap.v1': {
		label: 'SNMP v1 Trap',
		emptyPolicy: 'null'
	},
	'config.config.v1.communities': {
		label: 'Communities (v1)',
		widget: 'array-string',
		placeholder: 'Community string (e.g. public)'
	},
	'config.snmp_trap.v1.communities': {
		label: 'Communities (v1)',
		widget: 'array-string',
		placeholder: 'Community string (e.g. public)'
	},
	'config.config.v2c': {
		label: 'SNMP v2c Trap',
		defaultValue: { communities: ['public'] },
		emptyPolicy: 'null'
	},
	'config.snmp_trap.v2c': {
		label: 'SNMP v2c Trap',
		defaultValue: { communities: ['public'] },
		emptyPolicy: 'null'
	},
	'config.config.v2c.communities': {
		label: 'Communities (v2c)',
		widget: 'array-string',
		placeholder: 'Community string (e.g. public)',
		defaultValue: ['public'],
		validator: (_val, draft) => {
			const activeType = draft?.config?.type || draft?.type
			if (activeType && activeType !== 'snmp_trap') return
			const trapConfig = draft?.config?.config || draft?.config
			if (trapConfig) {
				const hasV1 =
					trapConfig.v1 &&
					Array.isArray(trapConfig.v1.communities) &&
					trapConfig.v1.communities.length > 0
				const hasV2c =
					trapConfig.v2c &&
					Array.isArray(trapConfig.v2c.communities) &&
					trapConfig.v2c.communities.length > 0
				const hasV3 =
					trapConfig.v3 &&
					Array.isArray(trapConfig.v3.security_models) &&
					trapConfig.v3.security_models.length > 0
				if (!hasV1 && !hasV2c && !hasV3) {
					return 'At least one SNMP version (v1, v2c, or v3) must be configured'
				}
			}
		}
	},
	'config.snmp_trap.v2c.communities': {
		label: 'Communities (v2c)',
		widget: 'array-string',
		placeholder: 'Community string (e.g. public)',
		defaultValue: ['public'],
		validator: (_val, draft) => {
			const activeType = draft?.config?.type || draft?.type
			if (activeType && activeType !== 'snmp_trap') return
			const trapConfig = draft?.config?.config || draft?.config
			if (trapConfig) {
				const hasV1 =
					trapConfig.v1 &&
					Array.isArray(trapConfig.v1.communities) &&
					trapConfig.v1.communities.length > 0
				const hasV2c =
					trapConfig.v2c &&
					Array.isArray(trapConfig.v2c.communities) &&
					trapConfig.v2c.communities.length > 0
				const hasV3 =
					trapConfig.v3 &&
					Array.isArray(trapConfig.v3.security_models) &&
					trapConfig.v3.security_models.length > 0
				if (!hasV1 && !hasV2c && !hasV3) {
					return 'At least one SNMP version (v1, v2c, or v3) must be configured'
				}
			}
		}
	},
	'config.config.v3': {
		label: 'SNMP v3 Trap',
		emptyPolicy: 'null'
	},
	'config.snmp_trap.v3': {
		label: 'SNMP v3 Trap',
		emptyPolicy: 'null'
	},
	'config.config.v3.security_models': {
		label: 'USM Security Models',
		widget: 'array-object',
		validator: (models, draft) => {
			const activeType = draft?.config?.type || draft?.type
			if (activeType && activeType !== 'snmp_trap') return
			if (Array.isArray(models)) {
				const issues: Record<string, string> = {}
				models.forEach((sm: any, idx: number) => {
					if (sm?.type === 'usm' && sm.config) {
						if (sm.config.privacy && !sm.config.authentication) {
							issues[
								`config.config.v3.security_models.${idx}.config.authentication.passphrase`
							] = 'Authentication is required when privacy is specified'
							issues[
								`config.snmp_trap.v3.security_models.${idx}.config.authentication.passphrase`
							] = 'Authentication is required when privacy is specified'
						}
					}
				})
				if (Object.keys(issues).length > 0) return issues
			}
		}
	},
	'config.snmp_trap.v3.security_models': {
		label: 'USM Security Models',
		widget: 'array-object',
		validator: (models, draft) => {
			const activeType = draft?.config?.type || draft?.type
			if (activeType && activeType !== 'snmp_trap') return
			if (Array.isArray(models)) {
				const issues: Record<string, string> = {}
				models.forEach((sm: any, idx: number) => {
					if (sm?.type === 'usm' && sm.config) {
						if (sm.config.privacy && !sm.config.authentication) {
							issues[
								`config.config.v3.security_models.${idx}.config.authentication.passphrase`
							] = 'Authentication is required when privacy is specified'
							issues[
								`config.snmp_trap.v3.security_models.${idx}.config.authentication.passphrase`
							] = 'Authentication is required when privacy is specified'
						}
					}
				})
				if (Object.keys(issues).length > 0) return issues
			}
		}
	},
	'config.config.v3.security_models.type': {
		label: 'Model Type',
		defaultValue: 'usm',
		disabled: true
	},
	'config.snmp_trap.v3.security_models.type': {
		label: 'Model Type',
		defaultValue: 'usm',
		disabled: true
	},
	'config.config.v3.security_models.config': {
		hideGroup: true
	},
	'config.snmp_trap.v3.security_models.config': {
		hideGroup: true
	},
	'config.config.v3.security_models.config.username': {
		label: 'Username / Security Name',
		placeholder: 'SNMP v3 username'
	},
	'config.snmp_trap.v3.security_models.config.username': {
		label: 'Username / Security Name',
		placeholder: 'SNMP v3 username'
	},
	'config.config.v3.security_models.config.context_name': {
		label: 'Context Name (Optional)',
		placeholder: 'SNMP context name'
	},
	'config.snmp_trap.v3.security_models.config.context_name': {
		label: 'Context Name (Optional)',
		placeholder: 'SNMP context name'
	},
	'config.config.v3.security_models.config.authentication': {
		label: 'Authentication',
		emptyPolicy: 'null'
	},
	'config.snmp_trap.v3.security_models.config.authentication': {
		label: 'Authentication',
		emptyPolicy: 'null'
	},
	'config.config.v3.security_models.config.authentication.protocol': {
		label: 'Protocol'
	},
	'config.snmp_trap.v3.security_models.config.authentication.protocol': {
		label: 'Protocol'
	},
	'config.config.v3.security_models.config.authentication.passphrase': {
		label: 'Passphrase',
		widget: 'password',
		sensitive: true,
		placeholder: 'Min 8 characters'
	},
	'config.snmp_trap.v3.security_models.config.authentication.passphrase': {
		label: 'Passphrase',
		widget: 'password',
		sensitive: true,
		placeholder: 'Min 8 characters'
	},
	'config.config.v3.security_models.config.privacy': {
		label: 'Privacy (Encryption)',
		emptyPolicy: 'null'
	},
	'config.snmp_trap.v3.security_models.config.privacy': {
		label: 'Privacy (Encryption)',
		emptyPolicy: 'null'
	},
	'config.config.v3.security_models.config.privacy.protocol': {
		label: 'Protocol'
	},
	'config.snmp_trap.v3.security_models.config.privacy.protocol': {
		label: 'Protocol'
	},
	'config.config.v3.security_models.config.privacy.passphrase': {
		label: 'Passphrase',
		widget: 'password',
		sensitive: true,
		placeholder: 'Min 8 characters'
	},
	'config.snmp_trap.v3.security_models.config.privacy.passphrase': {
		label: 'Passphrase',
		widget: 'password',
		sensitive: true,
		placeholder: 'Min 8 characters'
	},

	// Open Telemetry
	'config.config.mode': {
		label: 'Protocol Mode',
		defaultValue: 'otap',
		options: [
			{ label: 'OTAP (OpenTelemetry Arrow Protocol)', value: 'otap' },
			{ label: 'OTLP (Standard OpenTelemetry Protocol)', value: 'otlp' },
			{ label: 'Both (OTAP & OTLP)', value: 'both' }
		]
	},
	'config.open_telemetry.mode': {
		label: 'Protocol Mode',
		defaultValue: 'otap',
		options: [
			{ label: 'OTAP (OpenTelemetry Arrow Protocol)', value: 'otap' },
			{ label: 'OTLP (Standard OpenTelemetry Protocol)', value: 'otlp' },
			{ label: 'Both (OTAP & OTLP)', value: 'both' }
		]
	},
	'config.open_telemetry.batch_size': {
		label: 'Batch Size',
		group: 'Batch Configuration',
		defaultValue: 1000,
		placeholder: '1000'
	},
	'config.open_telemetry.batch_timeout': {
		label: 'Batch Timeout (ms)',
		group: 'Batch Configuration',
		defaultValue: 1000,
		placeholder: '1000'
	},
	'config.open_telemetry.listener': {
		label: 'TCP Listener Configuration'
	},
	'config.open_telemetry.listener.address': {
		label: 'Listener Address',
		placeholder: '0.0.0.0:4317',
		defaultValue: '0.0.0.0:4317'
	},
	'config.open_telemetry.listener.ip_whitelist': {
		label: 'IP Whitelist',
		widget: 'array-string',
		placeholder: 'e.g. 192.168.1.1'
	},
	'config.open_telemetry.listener.ip_blacklist': {
		label: 'IP Blacklist',
		widget: 'array-string',
		placeholder: 'e.g. 10.0.0.1'
	},
	'config.open_telemetry.tls': {
		label: 'TLS Encryption (Optional)',
		group: 'TLS Encryption',
		defaultValue: null,
		emptyPolicy: 'null'
	},
	'config.open_telemetry.tls.cert_path': {
		label: 'Certificate Path',
		placeholder: '/etc/ssl/certs/server.crt',
		validator: (certPath, draft) => {
			const activeType = draft?.config?.type || draft?.type
			if (activeType && activeType !== 'open_telemetry') return
			const otelConfig = draft?.config?.config || draft?.config
			if (otelConfig?.tls) {
				const hasCert = certPath && String(certPath).trim() !== ''
				const hasKey =
					otelConfig.tls.key_path && String(otelConfig.tls.key_path).trim() !== ''
				if (hasKey && !hasCert) {
					return 'Certificate path is required when private key path is provided'
				}
			}
		}
	},
	'config.open_telemetry.tls.key_path': {
		label: 'Private Key Path',
		placeholder: '/etc/ssl/private/server.key',
		validator: (keyPath, draft) => {
			const activeType = draft?.config?.type || draft?.type
			if (activeType && activeType !== 'open_telemetry') return
			const otelConfig = draft?.config?.config || draft?.config
			if (otelConfig?.tls) {
				const hasCert =
					otelConfig.tls.cert_path && String(otelConfig.tls.cert_path).trim() !== ''
				const hasKey = keyPath && String(keyPath).trim() !== ''
				if (hasCert && !hasKey) {
					return 'Private key path is required when certificate path is provided'
				}
			}
		}
	},
	'config.open_telemetry.tls.ca_cert_path': {
		label: 'CA Certificate Path (Optional)',
		placeholder: '/etc/ssl/certs/ca.crt'
	},
	'config.open_telemetry.tls.key_password': {
		label: 'Key Password (Optional)',
		widget: 'password',
		sensitive: true
	},
	'config.open_telemetry.tls.insecure_skip_verify': {
		label: 'Insecure Skip Verify',
		widget: 'switch',
		defaultValue: false
	},

	// Netflow
	'config.netflow.batch_size': {
		label: 'Batch Size',
		group: 'Batch Configuration',
		defaultValue: 1000,
		placeholder: '1000'
	},
	'config.netflow.batch_timeout': {
		label: 'Batch Timeout (ms)',
		group: 'Batch Configuration',
		defaultValue: 1000,
		placeholder: '1000'
	},
	'config.netflow.listener': {
		label: 'UDP Listener Configuration'
	},
	'config.netflow.listener.address': {
		label: 'Listener Address',
		placeholder: '0.0.0.0:2055',
		defaultValue: '0.0.0.0:2055'
	},
	'config.netflow.listener.buffer_cap': {
		label: 'Buffer Capacity',
		placeholder: '100'
	},
	'config.netflow.listener.max_packet_size': {
		label: 'Max Packet Size (bytes)',
		placeholder: '4096'
	},
	'config.netflow.listener.worker_count': {
		label: 'Worker Count',
		placeholder: '1'
	},
	'config.netflow.listener.ip_whitelist': {
		label: 'IP Whitelist',
		widget: 'array-string',
		placeholder: 'e.g. 192.168.1.1'
	},
	'config.netflow.listener.ip_blacklist': {
		label: 'IP Blacklist',
		widget: 'array-string',
		placeholder: 'e.g. 10.0.0.1'
	},

	// Syslog
	'config.syslog.batch_size': {
		label: 'Batch Size',
		group: 'Batch Configuration',
		defaultValue: 1000,
		placeholder: '1000'
	},
	'config.syslog.batch_timeout': {
		label: 'Batch Timeout (ms)',
		group: 'Batch Configuration',
		defaultValue: 1000,
		placeholder: '1000'
	},
	'config.syslog.listener': {
		label: 'Syslog Listener Configuration'
	},
	'config.syslog.listener.idle_timeout': {
		label: 'Idle Timeout (ms)',
		placeholder: '30000'
	},
	'config.syslog.listener.udp': {
		label: 'UDP Listener Configuration',
		defaultValue: { address: '0.0.0.0:514' }
	},
	'config.syslog.listener.udp.address': {
		label: 'UDP Listener Address',
		placeholder: '0.0.0.0:514',
		defaultValue: '0.0.0.0:514'
	},
	'config.syslog.listener.tcp.address': {
		label: 'TCP Listener Address',
		placeholder: '0.0.0.0:514',
		defaultValue: '0.0.0.0:514'
	},
	'config.syslog.listener.tls.address': {
		label: 'TLS Listener Address',
		placeholder: '0.0.0.0:6514',
		defaultValue: '0.0.0.0:6514'
	},
	'config.config.tls': {
		group: 'TLS Encryption',
		emptyPolicy: 'null'
	},
	'config.syslog.listener.tls': {
		group: 'TLS Encryption',
		defaultValue: null,
		emptyPolicy: 'null'
	}
}

// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { UiHint } from '@@/shared/types/dynamic-form.type'

export const sinkHints: Record<string, UiHint> = {
	name: { order: 1, label: 'Name', placeholder: 'Name of group' },
	adm_status: { order: 2, label: 'Status', widget: 'switch' },
	worker_count: { order: 3, label: 'Worker Count', placeholder: 'Worker Count' },
	labels: { order: 4, label: 'Labels', widget: 'label', emptyPolicy: 'empty-object' },
	inputs: { order: 5, label: 'Inputs' },
	batch: { order: 6, label: 'Batching Configuration', hideGroup: true },
	'batch.batch_size': { label: 'Batch Size', placeholder: 'Batch size' },
	'batch.batch_timeout': { label: 'Batch Timeout', placeholder: 'Batch timeout' },

	channel: {
		order: 7,
		label: 'Channel',
		discriminatorLabel: 'Chanel Type',
		discriminatorPlaceholder: 'Select chanel type',
		optionsOrder: ['disk', 'memory']
	},

	// Channel Disk
	'channel.disk.compression': {
		label: 'Compression Type',
		placeholder: 'Select compression type'
	},
	'channel.disk.max_disk_size': { label: 'Max Disk Size', placeholder: 'Max disk size' },
	'channel.disk.max_memtable_size': {
		label: 'Max Memtable Size',
		placeholder: 'Max memtable size'
	},
	'channel.disk.worker_count': { label: 'Worker Count', placeholder: 'Worker Count' },
	'channel.disk.non_blocking': { label: 'Non Blocking', widget: 'switch' },

	// Channel Memory
	'channel.memory.channel_buffer_size': {
		label: 'Channel Buffer Size',
		placeholder: 'channel buffer size'
	},
	'channel.memory.non_blocking': { label: 'Non Blocking', widget: 'switch' },

	config: {
		order: 8,
		label: 'Config',
		discriminatorLabel: 'Config Type',
		discriminatorPlaceholder: 'Select config type',
		optionsOrder: ['file', 'kafka', 'open_telemetry', 'clickhouse']
	},

	// Hide double nested borders on union branches
	'config.config': { hideGroup: true },

	// File
	'config.file.format': { label: 'Format', placeholder: 'Select Format' },
	'config.file.path': { label: 'Path', placeholder: 'Input path' },
	'config.file.throttle': { label: 'Throttle', placeholder: 'Input throttle' },

	// Kafka
	'config.kafka.topic': { label: 'Topic', placeholder: 'Input Topic' },
	'config.kafka.format': { label: 'Format', placeholder: 'Select Format' },
	'config.kafka.compression': { label: 'Compression', placeholder: 'Select Compression' },
	'config.kafka.balancer_strategy': {
		label: 'Balancer Strategy',
		placeholder: 'Select Balancer Strategy'
	},
	'config.kafka.required_acks': { label: 'Required Acks', placeholder: 'Select Required Acks' },
	'config.kafka.key_strategy': { label: 'Key Strategy', placeholder: 'Select Key Strategy' },
	'config.kafka.allow_empty_key': { label: 'Allow empty Key', widget: 'switch' },
	'config.kafka.write_timeout_seconds': {
		label: 'Write Timeout in Seconds',
		placeholder: 'Input Write Timeout in Seconds'
	},
	'config.kafka.hosts': { label: 'Hosts', placeholder: 'Input Host' },
	'config.kafka.security': { hideGroup: true },
	'config.kafka.security.protocol': {
		label: 'Security Protocol',
		placeholder: 'Select Security Protocol',
		options: [
			{ label: 'Plaintext', value: 'plaintext' },
			{ label: 'SSL', value: 'ssl' },
			{ label: 'SASL Plaintext', value: 'sasl_plaintext' },
			{ label: 'SASL SSL', value: 'sasl_ssl' }
		],
		validator: (protocol, draft) => {
			const activeType = draft?.config?.type || draft?.type
			if (activeType && activeType !== 'kafka') return
			const kafkaConfig = draft?.config?.config || draft?.config
			if (!kafkaConfig) return
			if (protocol === 'plaintext') {
				kafkaConfig.ssl = null
				kafkaConfig.sasl = null
			} else if (protocol === 'ssl') {
				kafkaConfig.sasl = null
			} else if (protocol === 'sasl_plaintext') {
				kafkaConfig.ssl = null
			}
		}
	},
	'config.kafka.ssl': {
		group: 'Kafka SSL Settings',
		defaultValue: null,
		emptyPolicy: 'null',
		visibleWhen:
			"(draft?.security?.protocol === 'ssl' || draft?.security?.protocol === 'sasl_ssl' || draft?.config?.security?.protocol === 'ssl' || draft?.config?.security?.protocol === 'sasl_ssl' || draft?.config?.config?.security?.protocol === 'ssl' || draft?.config?.config?.security?.protocol === 'ssl')",
		validator: (ssl, draft) => {
			const activeType = draft?.config?.type || draft?.type
			if (activeType && activeType !== 'kafka') return
			const kafkaConfig = draft?.config?.config || draft?.config
			const protocol = kafkaConfig?.security?.protocol || 'plaintext'
			if (protocol === 'ssl' || protocol === 'sasl_ssl') {
				if (!ssl) {
					return `Protocol '${protocol}' requires SSL encryption settings`
				}
				const hasCa = ssl.ca_cert_path && String(ssl.ca_cert_path).trim() !== ''
				const hasInsecure = Boolean(ssl.insecure_skip_verify)
				if (!hasCa && !hasInsecure) {
					return "CA certificate path or 'Skip TLS Verification' is required for SSL protocol"
				}
			}
		}
	},
	'config.kafka.ssl.ca_cert_path': {
		label: 'CA Certificate Path',
		placeholder: '/etc/ssl/certs/kafka-ca.pem'
	},
	'config.kafka.ssl.cert_path': {
		label: 'Client Certificate Path',
		placeholder: '/etc/ssl/certs/kafka-client.pem',
		validator: (certPath, draft) => {
			const activeType = draft?.config?.type || draft?.type
			if (activeType && activeType !== 'kafka') return
			const kafkaConfig = draft?.config?.config || draft?.config
			const ssl = kafkaConfig?.ssl
			if (ssl) {
				const hasCert = certPath && String(certPath).trim() !== ''
				const hasKey = ssl.key_path && String(ssl.key_path).trim() !== ''
				if (hasKey && !hasCert) {
					return 'Certificate path is required when private key path is provided'
				}
			}
		}
	},
	'config.kafka.ssl.key_path': {
		label: 'Client Private Key Path',
		placeholder: '/etc/ssl/private/kafka-client.key',
		validator: (keyPath, draft) => {
			const activeType = draft?.config?.type || draft?.type
			if (activeType && activeType !== 'kafka') return
			const kafkaConfig = draft?.config?.config || draft?.config
			const ssl = kafkaConfig?.ssl
			if (ssl) {
				const hasCert = ssl.cert_path && String(ssl.cert_path).trim() !== ''
				const hasKey = keyPath && String(keyPath).trim() !== ''
				if (hasCert && !hasKey) {
					return 'Private key path is required when client certificate path is provided'
				}
			}
		}
	},
	'config.kafka.ssl.key_password': {
		label: 'Private Key Password',
		widget: 'password',
		sensitive: true,
		placeholder: 'Optional key password'
	},
	'config.kafka.ssl.insecure_skip_verify': {
		label: 'Skip TLS Verification',
		widget: 'switch'
	},
	'config.kafka.sasl': {
		group: 'Kafka SASL Settings',
		defaultValue: null,
		emptyPolicy: 'null',
		visibleWhen:
			"(draft?.security?.protocol === 'sasl_plaintext' || draft?.security?.protocol === 'sasl_ssl' || draft?.config?.security?.protocol === 'sasl_plaintext' || draft?.config?.security?.protocol === 'sasl_ssl' || draft?.config?.config?.security?.protocol === 'sasl_plaintext' || draft?.config?.config?.security?.protocol === 'sasl_ssl')",
		validator: (sasl, draft) => {
			const activeType = draft?.config?.type || draft?.type
			if (activeType && activeType !== 'kafka') return
			const kafkaConfig = draft?.config?.config || draft?.config
			const protocol = kafkaConfig?.security?.protocol || 'plaintext'
			if (protocol === 'sasl_plaintext' || protocol === 'sasl_ssl') {
				const hasUsername = sasl?.username && String(sasl.username).trim() !== ''
				const hasPassword = sasl?.password && String(sasl.password).trim() !== ''
				if (!sasl || (!hasUsername && !hasPassword)) {
					return `Protocol '${protocol}' requires SASL authentication credentials`
				}
			}
		}
	},
	'config.kafka.sasl.mechanism': {
		label: 'SASL Mechanism',
		placeholder: 'Select SASL Mechanism',
		options: [
			{ label: 'Plain', value: 'plain' },
			{ label: 'SCRAM SHA-256', value: 'sha256' },
			{ label: 'SCRAM SHA-512', value: 'sha512' }
		]
	},
	'config.kafka.sasl.username': {
		label: 'Username',
		placeholder: 'Kafka SASL Username',
		validator: (username, draft) => {
			const activeType = draft?.config?.type || draft?.type
			if (activeType && activeType !== 'kafka') return
			const kafkaConfig = draft?.config?.config || draft?.config
			const protocol = kafkaConfig?.security?.protocol || 'plaintext'
			if (protocol === 'sasl_plaintext' || protocol === 'sasl_ssl') {
				if (!username || String(username).trim() === '') {
					return 'Username is required for SASL protocol'
				}
			}
		}
	},
	'config.kafka.sasl.password': {
		label: 'Password',
		widget: 'password',
		sensitive: true,
		placeholder: 'Kafka SASL Password',
		validator: (password, draft) => {
			const activeType = draft?.config?.type || draft?.type
			if (activeType && activeType !== 'kafka') return
			const kafkaConfig = draft?.config?.config || draft?.config
			const protocol = kafkaConfig?.security?.protocol || 'plaintext'
			if (protocol === 'sasl_plaintext' || protocol === 'sasl_ssl') {
				if (!password || String(password).trim() === '') {
					return 'Password is required for SASL protocol'
				}
			}
		}
	},

	// OpenTelemetry
	'config.open_telemetry.destinations': {
		label: 'Destinations',
		placeholder: 'Input Destination'
	},
	'config.open_telemetry.write_timeout': { label: 'Write Timeout', placeholder: '1000' },

	// ClickHouse
	'config.clickhouse.endpoints': { label: 'Endpoints', placeholder: 'Input Endpoint' },
	'config.clickhouse.database': { label: 'Database', placeholder: 'Input database' },
	'config.clickhouse.username': { label: 'Username', placeholder: 'Input username' },
	'config.clickhouse.password': {
		label: 'Password',
		widget: 'password',
		sensitive: true
	},
	'config.clickhouse.credentials.password': { widget: 'password', sensitive: true },
	'config.clickhouse.write_timeout': { label: 'Write Timeout', placeholder: '30000' },
	'config.clickhouse.async_insert': { label: 'Async Insert', widget: 'switch' },
	'config.clickhouse.wait_for_async_insert': {
		label: 'Wait For Async Insert',
		widget: 'switch'
	},
	'config.clickhouse.enable_schema_validation': {
		label: 'Enable Schema Validation',
		widget: 'switch'
	},
	'config.clickhouse.settings': {
		widget: 'record',
		label: 'ClickHouse Session Settings',
		description: 'Key-value settings like max_execution_time=60'
	}
}

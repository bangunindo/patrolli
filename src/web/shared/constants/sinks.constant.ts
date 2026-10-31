// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

export default {
	configs: {
		file: {
			path: '/var/log/patrolli/metrics.json',
			format: 'native_json',
			throttle: 0,
			batch_size: 1000,
			batch_timeout: 1000,
			non_blocking: false,
			channel_buffer_size: 0
		},
		kafka: {
			allow_empty_key: false,
			balancer_strategy: 'murmur2',
			batch_size: 1000,
			batch_timeout: 1000,
			channel_buffer_size: 0,
			compression: 'gzip',
			format: 'native_json',
			hosts: ['kafka-1:9093'],
			key_strategy: 'item_id',
			non_blocking: false,
			required_acks: 'all',
			sasl: null,
			security: {
				protocol: 'ssl'
			},
			ssl: {
				insecure_skip_verify: true,
				ca_cert_path: null,
				cert_path: null,
				key_path: null,
				key_password: null
			},
			topic: 'metrics',
			write_timeout_seconds: 30
		},
		patrolli: {
			batch_size: 1000,
			batch_timeout: 1000,
			channel_buffer_size: 0,
			compression: 'lz4',
			connection_count: 2,
			destinations: ['10.0.0.20:9000', '10.0.0.21:9000'],
			distribution: {
				strategy: 'sharded',
				key: 'host_id'
			},
			pre_shared_key: null,
			tls: {
				insecure_skip_verify: true,
				ca_cert_path: null,
				cert_path: null,
				key_path: null,
				key_password: null
			},
			non_blocking: false,
			write_timeout: 5000
		}
	}
}

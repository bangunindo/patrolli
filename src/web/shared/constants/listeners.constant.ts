// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

export default {
	configs: {
		dummy: {
			speed: 100,
			messages_per_batch: 500
		},
		netflow: {
			listener: {
				address: '0.0.0.0:2056',
				max_packet_size: 8192,
				buffer_cap: 200,
				worker_count: 2,
				ip_whitelist: ['192.0.2.10', '198.51.100.10'],
				ip_blacklist: []
			},
			batch: {
				batch_size: 500,
				batch_timeout: 1000
			}
		},
		snmp_trap: {
			listener: {
				address: '0.0.0.0:162',
				max_packet_size: 8192,
				buffer_cap: 200,
				worker_count: 2,
				ip_whitelist: ['203.0.113.10'],
				ip_blacklist: []
			},
			batch: {
				batch_size: 200,
				batch_timeout: 1000
			},
			use_mib: true,
			v1: {
				communities: ['public']
			},
			v2c: {
				communities: ['public', 'ops']
			},
			v3: {
				security_models: [
					{
						type: 'usm',
						config: {
							username: 'trap-user',
							context_name: 'snmp-ctx',
							authentication: {
								protocol: 'sha256',
								passphrase: 'authpass123'
							},
							privacy: {
								protocol: 'aes128',
								passphrase: 'privpass123'
							}
						}
					}
				]
			}
		},
		syslog: {
			listener: {
				udp: {
					address: '0.0.0.0:514',
					max_packet_size: 65536,
					buffer_cap: 200,
					worker_count: 1,
					ip_whitelist: ['198.51.100.20'],
					ip_blacklist: []
				},
				tcp: {
					address: '0.0.0.0:5514',
					ip_whitelist: [],
					ip_blacklist: []
				},
				idle_timeout: 600
			},
			batch: {
				batch_size: 1000,
				batch_timeout: 2000
			}
		},
		patrolli: {
			listener: {
				address: '0.0.0.0:4636',
				ip_whitelist: ['203.0.113.50'],
				ip_blacklist: []
			},
			pre_shared_key: 'shared-secret',
			batch: {
				batch_size: 1000,
				batch_timeout: 1000
			}
		}
	}
}

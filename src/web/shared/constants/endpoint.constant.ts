// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

export default {
	configs: {
		snmp_v1: {
			address: '192.0.2.10:161',
			community: 'public'
		},
		snmp_v2c: {
			address: '198.51.100.21:161',
			community: 'public',
			max_bulk_repetitions: 50,
			disable_bulk: false
		},
		snmp_v3: {
			address: '203.0.113.6:161',
			security_model: {
				type: 'usm',
				config: {
					username: 'snmp-user',
					context_name: 'bridge-ctx',
					authentication: {
						protocol: 'sha256',
						passphrase: 'authpass123'
					},
					privacy: {
						protocol: 'aes128',
						passphrase: 'privpass123'
					}
				}
			},
			max_bulk_repetitions: 25,
			disable_bulk: false
		}
	}
}

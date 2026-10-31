// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { UiHint } from '@@/shared/types/dynamic-form.type'

export const endpointHints: Record<string, UiHint> = {
	name: { order: 1, placeholder: 'Name of endpoint' },
	host_id: { order: 2, label: 'Host' },
	adm_status: { order: 3, label: 'Status', widget: 'switch' },
	config: {
		order: 4,
		discriminatorLabel: 'Protocol',
		discriminatorPlaceholder: 'Select endpoint protocol...'
	},

	// Hide double nested config borders on union branches
	'config.config': { hideGroup: true },

	// SNMP v1, v2c, v3 addresses & communities
	'config.config.address': { placeholder: '192.168.1.1:161', defaultValue: '' },
	'config.config.community': {
		widget: 'password',
		sensitive: true,
		placeholder: 'public',
		defaultValue: ''
	},
	'config.config.disable_bulk': { label: 'Disable Bulk' },
	'config.config.max_bulk_repetitions': { label: 'Max Bulk Repetitions', placeholder: '10' },

	// SNMP v3 USM credentials
	'config.config.security_model.config.authentication.passphrase': {
		widget: 'password',
		sensitive: true
	},
	'config.config.security_model.config.privacy.passphrase': {
		widget: 'password',
		sensitive: true
	},

	// HTTP
	'config.config.url': { placeholder: 'http://localhost:8080' },
	'config.config.connect_timeout': { label: 'Connect Timeout (ms)', placeholder: '1000' },
	'config.config.auth': {
		group: 'HTTP Authentication',
		discriminatorLabel: 'Authentication Type',
		discriminatorPlaceholder: 'Select authentication type...',
		discriminatorDefault: 'none',
		optionsOrder: ['none', 'Basic', 'Bearer']
	},
	'config.config.auth.Basic': { hideGroup: true },
	'config.config.auth.Basic.username': { label: 'Username', placeholder: 'Username' },
	'config.config.auth.Basic.password': {
		label: 'Password',
		widget: 'password',
		sensitive: true,
		placeholder: 'Password'
	},
	'config.config.auth.Bearer': {
		label: 'Bearer Token',
		widget: 'password',
		sensitive: true,
		placeholder: 'Enter Bearer Token...'
	},

	'config.config.tls': { group: 'TLS Configuration', hideGroup: true, emptyPolicy: 'null' },
	'config.config.tls.insecure_skip_verify': { label: 'Insecure Skip Verify', widget: 'switch' },
	'config.config.tls.ca_cert_path': {
		label: 'CA Certificate Path',
		placeholder: '/path/to/ca.pem'
	},
	'config.config.tls.cert_path': {
		label: 'Certificate Path',
		placeholder: '/path/to/cert.pem',
		validator: (certPath, draft) => {
			const httpConfig =
				draft?.config?.config || (draft?.config?.type === 'http' ? draft?.config : null)
			if (httpConfig?.tls) {
				const hasCert = certPath && String(certPath).trim() !== ''
				const hasKey =
					httpConfig.tls.key_path && String(httpConfig.tls.key_path).trim() !== ''
				if (hasKey && !hasCert) {
					return 'Certificate path is required when private key path is provided'
				}
			}
		}
	},
	'config.config.tls.key_path': {
		label: 'Private Key Path',
		placeholder: '/path/to/key.pem',
		validator: (keyPath, draft) => {
			const httpConfig =
				draft?.config?.config || (draft?.config?.type === 'http' ? draft?.config : null)
			if (httpConfig?.tls) {
				const hasCert =
					httpConfig.tls.cert_path && String(httpConfig.tls.cert_path).trim() !== ''
				const hasKey = keyPath && String(keyPath).trim() !== ''
				if (hasCert && !hasKey) {
					return 'Private key path is required when client certificate path is provided'
				}
			}
		}
	},
	'config.config.tls.key_password': {
		label: 'Private Key Password',
		widget: 'password',
		sensitive: true,
		placeholder: 'Key password'
	},

	// Backward-compatible path aliases
	'config.snmp_v1.community': { widget: 'password', sensitive: true, placeholder: 'public' },
	'config.snmp_v2c.community': { widget: 'password', sensitive: true, placeholder: 'public' },
	'config.snmp_v3.security_model.config.authentication.passphrase': {
		widget: 'password',
		sensitive: true
	},
	'config.snmp_v3.security_model.config.privacy.passphrase': {
		widget: 'password',
		sensitive: true
	},
	'config.http.connect_timeout': { label: 'Connect Timeout (ms)', placeholder: '1000' },
	'config.http.auth': {
		group: 'HTTP Authentication',
		discriminatorLabel: 'Authentication Type',
		discriminatorPlaceholder: 'Select authentication type...',
		discriminatorDefault: 'none',
		optionsOrder: ['none', 'Basic', 'Bearer']
	},
	'config.http.auth.Basic': { hideGroup: true },
	'config.http.auth.Basic.username': { label: 'Username', placeholder: 'Username' },
	'config.http.auth.Basic.password': {
		label: 'Password',
		widget: 'password',
		sensitive: true,
		placeholder: 'Password'
	},
	'config.http.auth.Bearer': {
		label: 'Bearer Token',
		widget: 'password',
		sensitive: true,
		placeholder: 'Enter Bearer Token...'
	},
	'config.http.tls': { group: 'TLS Configuration', hideGroup: true, emptyPolicy: 'null' },
	'config.http.tls.insecure_skip_verify': { label: 'Insecure Skip Verify', widget: 'switch' },
	'config.http.tls.ca_cert_path': {
		label: 'CA Certificate Path',
		placeholder: '/path/to/ca.pem'
	},
	'config.http.tls.cert_path': {
		label: 'Certificate Path',
		placeholder: '/path/to/cert.pem',
		validator: (certPath, draft) => {
			const httpConfig =
				draft?.config?.config || (draft?.config?.type === 'http' ? draft?.config : null)
			if (httpConfig?.tls) {
				const hasCert = certPath && String(certPath).trim() !== ''
				const hasKey =
					httpConfig.tls.key_path && String(httpConfig.tls.key_path).trim() !== ''
				if (hasKey && !hasCert) {
					return 'Certificate path is required when private key path is provided'
				}
			}
		}
	},
	'config.http.tls.key_path': {
		label: 'Private Key Path',
		placeholder: '/path/to/key.pem',
		validator: (keyPath, draft) => {
			const httpConfig =
				draft?.config?.config || (draft?.config?.type === 'http' ? draft?.config : null)
			if (httpConfig?.tls) {
				const hasCert =
					httpConfig.tls.cert_path && String(httpConfig.tls.cert_path).trim() !== ''
				const hasKey = keyPath && String(keyPath).trim() !== ''
				if (hasCert && !hasKey) {
					return 'Private key path is required when client certificate path is provided'
				}
			}
		}
	},
	'config.http.tls.key_password': {
		label: 'Private Key Password',
		widget: 'password',
		sensitive: true,
		placeholder: 'Key password'
	}
}

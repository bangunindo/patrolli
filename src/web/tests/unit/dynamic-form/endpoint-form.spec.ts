// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from 'vitest'
import { CreateEndpointRequest } from '@@/shared/generated/patrolliAPI.schemas.type'
import { introspectZodSchema } from '@/app/services/forms/zod-introspector'
import { AutoFormAdapter } from '@/app/services/forms/auto-adapter'
import { endpointHints } from '@/app/services/forms/ui-hints/endpoint.hints'

describe('Pilot Migration: Endpoint Form Dynamic Schema Tests', () => {
	it('menghasilkan field descriptors lengkap untuk Endpoint dengan UI Hints terpasang', () => {
		const descriptors = introspectZodSchema(CreateEndpointRequest, [], endpointHints)

		expect(descriptors.length).toBe(4)

		const nameDesc = descriptors.find((d) => d.name === 'name')!
		expect(nameDesc.widget).toBe('text')
		expect(nameDesc.required).toBe(true)
		expect(nameDesc.placeholder).toBe('Name of endpoint')

		const hostDesc = descriptors.find((d) => d.name === 'host_id')!
		expect(hostDesc.label).toBe('Host')
		expect(hostDesc.required).toBe(true)

		const statusDesc = descriptors.find((d) => d.name === 'adm_status')!
		expect(statusDesc.widget).toBe('switch')
		expect(statusDesc.label).toBe('Status')

		const configDesc = descriptors.find((d) => d.name === 'config')!
		expect(configDesc.widget).toBe('discriminated-union')
		expect(configDesc.unionDiscriminator).toBe('type')
		expect(configDesc.options?.map((o) => o.value)).toEqual([
			'snmp_v1',
			'snmp_v2c',
			'snmp_v3',
			'http'
		])

		// Verify HTTP branch
		const httpBranch = configDesc.unionBranches?.['http']
		expect(httpBranch).toBeDefined()
		const httpConfigDesc = httpBranch?.find((d) => d.name === 'config')
		expect(httpConfigDesc).toBeDefined()

		// Check sub-fields inside HTTP config
		const urlDesc = httpConfigDesc?.children?.find((d) => d.name === 'url')
		expect(urlDesc).toBeDefined()
		expect(urlDesc?.widget).toBe('text')
		expect(urlDesc?.required).toBe(true)

		const timeoutDesc = httpConfigDesc?.children?.find((d) => d.name === 'connect_timeout')
		expect(timeoutDesc).toBeDefined()
		expect(timeoutDesc?.widget).toBe('number')
		expect(timeoutDesc?.defaultValue).toBe(1000)

		// Check auth field as externally-tagged union with none, Basic, Bearer options
		const authDesc = httpConfigDesc?.children?.find((d) => d.name === 'auth')
		expect(authDesc).toBeDefined()
		expect(authDesc?.widget).toBe('discriminated-union')
		expect(authDesc?.unionTagMode).toBe('external')
		expect(authDesc?.required).toBe(false)
		expect(authDesc?.isNullable).toBe(true)
		expect(authDesc?.options?.map((o) => o.value)).toEqual(['none', 'Basic', 'Bearer'])

		// Basic branch in auth has username and password (masked)
		const basicBranch = authDesc?.unionBranches?.['Basic']
		expect(basicBranch).toBeDefined()
		const basicObjDesc = basicBranch?.[0]
		expect(basicObjDesc?.name).toBe('Basic')
		const usernameDesc = basicObjDesc?.children?.find((d) => d.name === 'username')
		expect(usernameDesc).toBeDefined()
		const passwordDesc = basicObjDesc?.children?.find((d) => d.name === 'password')
		expect(passwordDesc).toBeDefined()
		expect(passwordDesc?.widget).toBe('password')

		// Bearer branch in auth has single Bearer input (masked)
		const bearerBranch = authDesc?.unionBranches?.['Bearer']
		expect(bearerBranch).toBeDefined()
		const bearerDesc = bearerBranch?.[0]
		expect(bearerDesc?.name).toBe('Bearer')
		expect(bearerDesc?.widget).toBe('password')

		// Check tls field as optional object
		const tlsDesc = httpConfigDesc?.children?.find((d) => d.name === 'tls')
		expect(tlsDesc).toBeDefined()
		expect(tlsDesc?.required).toBe(false)
		expect(tlsDesc?.isNullable).toBe(true)
		const skipVerifyDesc = tlsDesc?.children?.find((d) => d.name === 'insecure_skip_verify')
		expect(skipVerifyDesc?.widget).toBe('switch')
	})

	describe('Validasi Varian Protokol Endpoint via AutoFormAdapter', () => {
		const baseEndpoint = {
			name: 'Core Router RouterOS',
			host_id: '11111111-1111-1111-1111-111111111111',
			adm_status: 1
		}

		it('berhasil memvalidasi protokol SNMP v1', () => {
			const draft = {
				...baseEndpoint,
				config: {
					type: 'snmp_v1',
					config: {
						address: '192.168.1.1:161',
						community: 'public'
					}
				}
			}

			const result = AutoFormAdapter.toRequest(CreateEndpointRequest, draft, endpointHints)
			expect(result.success).toBe(true)
			if (result.success) {
				expect(result.data.name).toBe('Core Router RouterOS')
				expect(result.data.config.type).toBe('snmp_v1')
			}
		})

		it('berhasil memvalidasi protokol SNMP v2c', () => {
			const draft = {
				...baseEndpoint,
				config: {
					type: 'snmp_v2c',
					config: {
						address: '192.168.1.2:161',
						community: 'private',
						disable_bulk: true,
						max_bulk_repetitions: 20
					}
				}
			}

			const result = AutoFormAdapter.toRequest(CreateEndpointRequest, draft, endpointHints)
			expect(result.success).toBe(true)
			if (result.success) {
				expect(result.data.config.type).toBe('snmp_v2c')
			}
		})

		it('berhasil memvalidasi protokol SNMP v3 dengan USM', () => {
			const draft = {
				...baseEndpoint,
				config: {
					type: 'snmp_v3',
					config: {
						address: '192.168.1.3:161',
						disable_bulk: false,
						security_model: {
							type: 'usm',
							config: {
								username: 'snmpadmin',
								context_name: 'context1',
								authentication: {
									protocol: 'sha256',
									passphrase: 'supersecretpass'
								}
							}
						}
					}
				}
			}

			const result = AutoFormAdapter.toRequest(CreateEndpointRequest, draft, endpointHints)
			expect(result.success).toBe(true)
			if (result.success) {
				expect(result.data.config.type).toBe('snmp_v3')
			}
		})

		it('berhasil memvalidasi protokol HTTP dengan connect_timeout tanpa auth', () => {
			const draft = {
				...baseEndpoint,
				config: {
					type: 'http',
					config: {
						url: 'https://api.patrolli.internal/metrics',
						connect_timeout: 5000,
						auth: null,
						tls: null
					}
				}
			}

			const result = AutoFormAdapter.toRequest(CreateEndpointRequest, draft, endpointHints)
			expect(result.success).toBe(true)
			if (result.success) {
				expect(result.data.config.type).toBe('http')
			}
		})

		it('berhasil memvalidasi protokol HTTP dengan Basic Auth dan TLS', () => {
			const draft = {
				...baseEndpoint,
				config: {
					type: 'http',
					config: {
						url: 'https://api.patrolli.internal/metrics',
						connect_timeout: 2000,
						auth: {
							Basic: {
								username: 'admin',
								password: 'supersecretpassword'
							}
						},
						tls: {
							insecure_skip_verify: true
						}
					}
				}
			}

			const result = AutoFormAdapter.toRequest(CreateEndpointRequest, draft, endpointHints)
			expect(result.success).toBe(true)
			if (result.success) {
				expect(result.data.config.type).toBe('http')
				expect((result.data.config.config as any).auth).toEqual({
					Basic: {
						username: 'admin',
						password: 'supersecretpassword'
					}
				})
				expect((result.data.config.config as any).tls?.insecure_skip_verify).toBe(true)
			}
		})

		it('berhasil memvalidasi protokol HTTP dengan Bearer Auth', () => {
			const draft = {
				...baseEndpoint,
				config: {
					type: 'http',
					config: {
						url: 'https://api.patrolli.internal/metrics',
						connect_timeout: 3000,
						auth: {
							Bearer: 'my-bearer-token-12345'
						}
					}
				}
			}

			const result = AutoFormAdapter.toRequest(CreateEndpointRequest, draft, endpointHints)
			expect(result.success).toBe(true)
			if (result.success) {
				expect(result.data.config.type).toBe('http')
				expect((result.data.config.config as any).auth).toEqual({
					Bearer: 'my-bearer-token-12345'
				})
			}
		})

		it('menolak payload HTTP jika Basic Auth kredensial kosong', () => {
			const draft = {
				...baseEndpoint,
				config: {
					type: 'http',
					config: {
						url: 'https://api.patrolli.internal/metrics',
						auth: {
							Basic: {
								username: '',
								password: ''
							}
						}
					}
				}
			}

			const result = AutoFormAdapter.toRequest(CreateEndpointRequest, draft, endpointHints)
			expect(result.success).toBe(false)
			if (!result.success) {
				// Issues must appear on Basic auth username and password fields
				const hasUsernameIssue =
					!!result.issues['config.config.auth.Basic.username'] ||
					!!result.issues['config.http.auth.Basic.username']
				const hasPasswordIssue =
					!!result.issues['config.config.auth.Basic.password'] ||
					!!result.issues['config.http.auth.Basic.password']
				expect(hasUsernameIssue).toBe(true)
				expect(hasPasswordIssue).toBe(true)
			}
		})

		it('menolak payload HTTP jika Bearer token kosong', () => {
			const draft = {
				...baseEndpoint,
				config: {
					type: 'http',
					config: {
						url: 'https://api.patrolli.internal/metrics',
						auth: {
							Bearer: ''
						}
					}
				}
			}

			const result = AutoFormAdapter.toRequest(CreateEndpointRequest, draft, endpointHints)
			expect(result.success).toBe(false)
			if (!result.success) {
				const hasBearerIssue =
					!!result.issues['config.config.auth.Bearer'] ||
					!!result.issues['config.http.auth.Bearer']
				expect(hasBearerIssue).toBe(true)
			}
		})

		it('menolak payload HTTP jika url kosong', () => {
			const draft = {
				...baseEndpoint,
				config: {
					type: 'http',
					config: {
						url: ''
					}
				}
			}

			const result = AutoFormAdapter.toRequest(CreateEndpointRequest, draft, endpointHints)
			expect(result.success).toBe(false)
			if (!result.success) {
				const hasUrlIssue =
					!!result.issues['config.config.url'] || !!result.issues['config.http.url']
				expect(hasUrlIssue).toBe(true)
			}
		})

		it('menolak payload jika ada field wajib yang kosong', () => {
			const draft = {
				name: '',
				host_id: '',
				config: {
					type: 'snmp_v1',
					config: {
						address: '',
						community: ''
					}
				}
			}

			const result = AutoFormAdapter.toRequest(CreateEndpointRequest, draft, endpointHints)
			expect(result.success).toBe(false)
			if (!result.success) {
				expect(result.issues['name']).toBeDefined()
				expect(result.issues['host_id']).toBeDefined()
			}
		})

		it('AutoFormAdapter.toDraft menghasilkan default endpoint form yang lengkap', () => {
			const draft = AutoFormAdapter.toDraft(CreateEndpointRequest, null, endpointHints)
			expect(draft.adm_status).toBe(1)
			expect(draft.config.type).toBe('snmp_v1')
			expect(draft.config.config.address).toBeDefined()
			expect(draft.config.config.community).toBeDefined()
		})

		it('AutoFormAdapter.toDraft mempopulasi data edit mode untuk HTTP endpoint dengan auth', () => {
			const existing = {
				name: 'Existing HTTP',
				host_id: '11111111-1111-1111-1111-111111111111',
				adm_status: 1,
				config: {
					type: 'http',
					config: {
						url: 'https://example.com/api',
						connect_timeout: 3000,
						auth: {
							Basic: {
								username: 'myuser',
								password: 'mypassword'
							}
						},
						tls: null
					}
				}
			}

			const draft = AutoFormAdapter.toDraft(CreateEndpointRequest, existing, endpointHints)
			expect(draft.name).toBe('Existing HTTP')
			expect(draft.config.type).toBe('http')
			expect(draft.config.config.url).toBe('https://example.com/api')
			expect(draft.config.config.auth.Basic.username).toBe('myuser')
			expect(draft.config.config.auth.Basic.password).toBe('mypassword')
		})

		it('menolak payload HTTP jika client cert_path diisi tanpa key_path', () => {
			const draft = {
				...baseEndpoint,
				config: {
					type: 'http',
					config: {
						url: 'https://api.patrolli.internal/metrics',
						tls: {
							cert_path: '/etc/ssl/client.crt',
							key_path: ''
						}
					}
				}
			}

			const result = AutoFormAdapter.toRequest(CreateEndpointRequest, draft, endpointHints)
			expect(result.success).toBe(false)
			if (!result.success) {
				const hasKeyIssue =
					!!result.issues['config.config.tls.key_path'] ||
					!!result.issues['config.http.tls.key_path']
				expect(hasKeyIssue).toBe(true)
			}
		})

		it('menolak payload HTTP jika client key_path diisi tanpa cert_path', () => {
			const draft = {
				...baseEndpoint,
				config: {
					type: 'http',
					config: {
						url: 'https://api.patrolli.internal/metrics',
						tls: {
							cert_path: '',
							key_path: '/etc/ssl/client.key'
						}
					}
				}
			}

			const result = AutoFormAdapter.toRequest(CreateEndpointRequest, draft, endpointHints)
			expect(result.success).toBe(false)
			if (!result.success) {
				const hasCertIssue =
					!!result.issues['config.config.tls.cert_path'] ||
					!!result.issues['config.http.tls.cert_path']
				expect(hasCertIssue).toBe(true)
			}
		})

		it('AutoFormAdapter.toRequest membersihkan TLS yang kosong menjadi null pada HTTP endpoint', () => {
			const draft = {
				...baseEndpoint,
				config: {
					type: 'http',
					config: {
						url: 'https://api.patrolli.internal/metrics',
						connect_timeout: 1000,
						tls: {
							ca_cert_path: '',
							cert_path: '',
							key_path: '',
							key_password: '',
							insecure_skip_verify: false
						}
					}
				}
			}

			const result = AutoFormAdapter.toRequest(CreateEndpointRequest, draft, endpointHints)
			expect(result.success).toBe(true)
			if (result.success) {
				expect((result.data.config.config as any).tls).toBeNull()
			}
		})
	})
})

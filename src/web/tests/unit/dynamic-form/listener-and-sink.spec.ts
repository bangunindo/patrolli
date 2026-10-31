// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import {
	CreateHostItemRequest,
	CreateListenerRequest,
	CreateSinkRequest,
	ItemConfig
} from '@@/shared/generated/patrolliAPI.schemas.type'
import { introspectField, introspectZodSchema } from '@/app/services/forms/zod-introspector'
import { itemHints } from '@/app/services/forms/ui-hints/item.hints'
import { listenerHints } from '@/app/services/forms/ui-hints/listener.hints'
import { sinkHints } from '@/app/services/forms/ui-hints/sink.hints'
import { AutoFormAdapter } from '@/app/services/forms/auto-adapter'
import SinkForm from '@/components/configuration/sinks/Form.vue'
import AppInputSelect from '@/components/app/input/AppInputSelect.vue'

setActivePinia(createPinia())

describe('Collector Module Listener & Sink Form Dynamic Form Integration', () => {
	describe('Collector Module: Listener', () => {
		it('mengintrospeksi ItemConfig listener dengan discriminator method pada match_strategy', () => {
			const desc = introspectField('config', ItemConfig, [], itemHints)
			expect(desc.widget).toBe('discriminated-union')
			expect(desc.unionDiscriminator).toBe('type')

			const listenerBranch = desc.unionBranches?.['listener']
			expect(listenerBranch).toBeDefined()

			// Listener branch has config (Listener object)
			const listenerConfigDesc = listenerBranch?.find((d) => d.name === 'config')
			expect(listenerConfigDesc).toBeDefined()

			// Inside Listener object: match_strategy
			const matchStrategyDesc = listenerConfigDesc?.children?.find(
				(d) => d.name === 'match_strategy'
			)
			expect(matchStrategyDesc).toBeDefined()
			expect(matchStrategyDesc?.widget).toBe('discriminated-union')
			expect(matchStrategyDesc?.unionDiscriminator).toBe('method')

			// Options order follows hints: none, source_ips, endpoint_source_ip, attribute_match
			const optionValues = matchStrategyDesc?.options?.map((o) => o.value)
			expect(optionValues).toEqual([
				'none',
				'source_ips',
				'endpoint_source_ip',
				'attribute_match'
			])

			// source_ips branch has config: array-string
			const sourceIpsBranch = matchStrategyDesc?.unionBranches?.['source_ips']
			expect(sourceIpsBranch).toBeDefined()
			const sourceIpsConfig = sourceIpsBranch?.find((d) => d.name === 'config')
			expect(sourceIpsConfig?.widget).toBe('array-string')
			expect(sourceIpsConfig?.label).toBe('Source IPs')

			// attribute_match branch has config: attribute-filter
			const attrMatchBranch = matchStrategyDesc?.unionBranches?.['attribute_match']
			expect(attrMatchBranch).toBeDefined()
			const attrMatchConfig = attrMatchBranch?.find((d) => d.name === 'config')
			expect(attrMatchConfig?.widget).toBe('attribute-filter')
			expect(attrMatchConfig?.label).toBe('Match Filter')
		})

		it('AutoFormAdapter validasi draft Host Item dengan match_strategy source_ips', () => {
			const draft = {
				name: 'Listener Item',
				schedule: '10s',
				timeout_ms: 1000,
				flags: 0,
				adm_status: 1,
				config: {
					type: 'listener',
					config: {
						match_strategy: {
							method: 'source_ips',
							config: ['192.168.1.1', '10.0.0.1']
						}
					}
				}
			}

			const res = AutoFormAdapter.toRequest(CreateHostItemRequest, draft, itemHints)
			expect(res.success).toBe(true)
			if (res.success) {
				expect(res.data.config).toEqual({
					type: 'listener',
					config: {
						match_strategy: {
							method: 'source_ips',
							config: ['192.168.1.1', '10.0.0.1']
						}
					}
				})
			}
		})

		it('AutoFormAdapter validasi draft Host Item dengan match_strategy none', () => {
			const draft = {
				name: 'Listener Item None',
				schedule: '10s',
				timeout_ms: 1000,
				flags: 0,
				adm_status: 1,
				config: {
					type: 'listener',
					config: {
						match_strategy: {
							method: 'none'
						}
					}
				}
			}

			const res = AutoFormAdapter.toRequest(CreateHostItemRequest, draft, itemHints)
			expect(res.success).toBe(true)
			if (res.success) {
				expect(res.data.config).toEqual({
					type: 'listener',
					config: {
						match_strategy: {
							method: 'none'
						}
					}
				})
			}
		})
	})

	describe('Sink Form: CreateSinkRequest', () => {
		it('mengintrospeksi CreateSinkRequest dengan channel (ZodIntersection) dan config', () => {
			const descriptors = introspectZodSchema(CreateSinkRequest, [], sinkHints)

			// Channel descriptor
			const channelDesc = descriptors.find((d) => d.name === 'channel')
			expect(channelDesc).toBeDefined()
			expect(channelDesc?.widget).toBe('discriminated-union')
			expect(channelDesc?.unionDiscriminator).toBe('type')

			const channelOptions = channelDesc?.options?.map((o) => o.value)
			expect(channelOptions).toEqual(['disk', 'memory'])

			// Disk branch properties
			const diskBranch = channelDesc?.unionBranches?.['disk']
			expect(diskBranch).toBeDefined()
			const diskCompression = diskBranch?.find((d) => d.name === 'compression')
			expect(diskCompression).toBeDefined()
			expect(diskCompression?.label).toBe('Compression Type')

			// Memory branch properties
			const memoryBranch = channelDesc?.unionBranches?.['memory']
			expect(memoryBranch).toBeDefined()
			const memBufferSize = memoryBranch?.find((d) => d.name === 'channel_buffer_size')
			expect(memBufferSize).toBeDefined()
			expect(memBufferSize?.label).toBe('Channel Buffer Size')

			// Config descriptor
			const configDesc = descriptors.find((d) => d.name === 'config')
			expect(configDesc).toBeDefined()
			expect(configDesc?.widget).toBe('discriminated-union')
			expect(configDesc?.unionDiscriminator).toBe('type')

			const configOptions = configDesc?.options?.map((o) => o.value)
			expect(configOptions).toEqual(['file', 'kafka', 'open_telemetry', 'clickhouse'])

			// Kafka branch
			const kafkaBranch = configDesc?.unionBranches?.['kafka']
			expect(kafkaBranch).toBeDefined()
			const kafkaConfig = kafkaBranch?.find((d) => d.name === 'config')
			expect(kafkaConfig).toBeDefined()
			const kafkaTopic = kafkaConfig?.children?.find((d) => d.name === 'topic')
			expect(kafkaTopic?.label).toBe('Topic')

			// Protocol options
			const securityDesc = kafkaConfig?.children?.find((d) => d.name === 'security')
			const protocolDesc = securityDesc?.children?.find((d) => d.name === 'protocol')
			expect(protocolDesc).toBeDefined()
			expect(protocolDesc?.label).toBe('Security Protocol')
			expect(protocolDesc?.options?.map((o) => o.value)).toEqual([
				'plaintext',
				'ssl',
				'sasl_plaintext',
				'sasl_ssl'
			])

			// SSL and SASL visibleWhen
			const sslDesc = kafkaConfig?.children?.find((d) => d.name === 'ssl')
			expect(sslDesc).toBeDefined()
			expect(sslDesc?.hints?.group).toBe('Kafka SSL Settings')
			expect(sslDesc?.hints?.visibleWhen).toBeDefined()
			expect(typeof sslDesc?.hints?.visibleWhen).toBe('string')
			expect(sslDesc?.hints?.visibleWhen).toContain("'ssl'")
			expect(sslDesc?.hints?.visibleWhen).toContain("'sasl_ssl'")

			const saslDesc = kafkaConfig?.children?.find((d) => d.name === 'sasl')
			expect(saslDesc).toBeDefined()
			expect(saslDesc?.hints?.group).toBe('Kafka SASL Settings')
			expect(saslDesc?.hints?.visibleWhen).toBeDefined()
			expect(typeof saslDesc?.hints?.visibleWhen).toBe('string')
			expect(saslDesc?.hints?.visibleWhen).toContain("'sasl_plaintext'")
			expect(saslDesc?.hints?.visibleWhen).toContain("'sasl_ssl'")
		})

		it('AutoFormAdapter validasi draft CreateSinkRequest', () => {
			const draft = {
				name: 'Sink Production',
				adm_status: 1,
				worker_count: 2,
				batch: {
					batch_size: 1000,
					batch_timeout: 1000
				},
				channel: {
					type: 'disk',
					compression: 'none',
					max_disk_size: 1000000,
					max_memtable_size: 500000,
					non_blocking: false,
					worker_count: 1
				},
				config: {
					type: 'file',
					config: {
						path: '/tmp/output.log',
						throttle: 10
					}
				}
			}

			const res = AutoFormAdapter.toRequest(CreateSinkRequest, draft, sinkHints)
			expect(res.success).toBe(true)
			if (res.success) {
				expect(res.data.channel.type).toBe('disk')
				expect(res.data.config.type).toBe('file')
			}
		})

		it('mengintrospeksi optional union distribution pada ClickHouse sink dengan opsi none', () => {
			const desc = introspectField('config', CreateSinkRequest.shape.config, [], sinkHints)
			const clickhouseBranch = desc.unionBranches?.['clickhouse']
			expect(clickhouseBranch).toBeDefined()
			const clickhouseConfig = clickhouseBranch?.find((d) => d.name === 'config')
			expect(clickhouseConfig).toBeDefined()
			const distDesc = clickhouseConfig?.children?.find((d) => d.name === 'distribution')
			expect(distDesc).toBeDefined()
			expect(distDesc?.widget).toBe('discriminated-union')
			const optValues = distDesc?.options?.map((o) => o.value)
			expect(optValues).toContain('none')
		})

		it('AutoFormAdapter.toRequest memvalidasi Kafka Sink dengan protocol plaintext dan membersihkan ssl & sasl ke null', () => {
			const draft = {
				name: 'Kafka Plaintext Sink',
				adm_status: 1,
				worker_count: 1,
				labels: '',
				batch: {
					batch_size: 1000,
					batch_timeout: 1000
				},
				channel: {
					type: 'memory',
					channel_buffer_size: 1000,
					non_blocking: false
				},
				config: {
					type: 'kafka',
					config: {
						hosts: ['kafka-broker:9092'],
						topic: 'events',
						format: 'json_otlp_batch',
						security: {
							protocol: 'plaintext'
						},
						ssl: {
							ca_cert_path: '/etc/ssl/ca.pem',
							insecure_skip_verify: false
						},
						sasl: {
							mechanism: 'plain',
							username: 'user',
							password: 'pass'
						}
					}
				}
			}

			const res = AutoFormAdapter.toRequest(CreateSinkRequest, draft, sinkHints)
			expect(res.success).toBe(true)
			if (res.success) {
				const cfg = (res.data.config as any).config
				expect(cfg.security.protocol).toBe('plaintext')
				expect(cfg.ssl).toBeNull()
				expect(cfg.sasl).toBeNull()
			}
		})

		it('AutoFormAdapter.toRequest memvalidasi Kafka Sink dengan protocol ssl', () => {
			const validSslDraft = {
				name: 'Kafka SSL Sink',
				adm_status: 1,
				worker_count: 1,
				labels: '',
				batch: {
					batch_size: 1000,
					batch_timeout: 1000
				},
				channel: {
					type: 'memory',
					channel_buffer_size: 1000,
					non_blocking: false
				},
				config: {
					type: 'kafka',
					config: {
						hosts: ['kafka-broker:9093'],
						topic: 'secure-events',
						format: 'json_otlp_batch',
						security: {
							protocol: 'ssl'
						},
						ssl: {
							ca_cert_path: '/etc/ssl/ca.pem',
							cert_path: '/etc/ssl/client.pem',
							key_path: '/etc/ssl/client.key',
							insecure_skip_verify: false
						},
						sasl: {
							mechanism: 'plain',
							username: 'should_be_cleared',
							password: 'pwd'
						}
					}
				}
			}

			const res = AutoFormAdapter.toRequest(CreateSinkRequest, validSslDraft, sinkHints)
			expect(res.success).toBe(true)
			if (res.success) {
				const cfg = (res.data.config as any).config
				expect(cfg.security.protocol).toBe('ssl')
				expect(cfg.ssl).not.toBeNull()
				expect(cfg.ssl.ca_cert_path).toBe('/etc/ssl/ca.pem')
				expect(cfg.sasl).toBeNull()
			}

			// SSL tanpa ssl block atau ca_cert_path ditolak
			const missingSslDraft = {
				...validSslDraft,
				config: {
					type: 'kafka',
					config: {
						...validSslDraft.config.config,
						ssl: null
					}
				}
			}
			const resMissing = AutoFormAdapter.toRequest(
				CreateSinkRequest,
				missingSslDraft,
				sinkHints
			)
			expect(resMissing.success).toBe(false)
			if (!resMissing.success) {
				expect(resMissing.issues['config.config.ssl']).toContain(
					"Protocol 'ssl' requires SSL encryption"
				)
			}
		})

		it('AutoFormAdapter.toRequest memvalidasi Kafka Sink dengan protocol sasl_plaintext', () => {
			const validSaslDraft = {
				name: 'Kafka SASL Sink',
				adm_status: 1,
				worker_count: 1,
				labels: '',
				batch: {
					batch_size: 1000,
					batch_timeout: 1000
				},
				channel: {
					type: 'memory',
					channel_buffer_size: 1000,
					non_blocking: false
				},
				config: {
					type: 'kafka',
					config: {
						hosts: ['kafka-broker:9094'],
						topic: 'auth-events',
						format: 'json_otlp_batch',
						security: {
							protocol: 'sasl_plaintext'
						},
						sasl: {
							mechanism: 'sha256',
							username: 'admin',
							password: 'secretpassword'
						},
						ssl: {
							ca_cert_path: '/etc/ssl/ca.pem'
						}
					}
				}
			}

			const res = AutoFormAdapter.toRequest(CreateSinkRequest, validSaslDraft, sinkHints)
			expect(res.success).toBe(true)
			if (res.success) {
				const cfg = (res.data.config as any).config
				expect(cfg.security.protocol).toBe('sasl_plaintext')
				expect(cfg.sasl.mechanism).toBe('sha256')
				expect(cfg.sasl.username).toBe('admin')
				expect(cfg.ssl).toBeNull()
			}

			// SASL tanpa credentials ditolak
			const invalidSaslDraft = {
				...validSaslDraft,
				config: {
					type: 'kafka',
					config: {
						...validSaslDraft.config.config,
						sasl: {
							mechanism: 'plain',
							username: '',
							password: ''
						}
					}
				}
			}
			const resInvalid = AutoFormAdapter.toRequest(
				CreateSinkRequest,
				invalidSaslDraft,
				sinkHints
			)
			expect(resInvalid.success).toBe(false)
			if (!resInvalid.success) {
				expect(resInvalid.issues['config.config.sasl']).toBeDefined()
			}
		})

		it('AutoFormAdapter.toRequest memvalidasi Kafka Sink dengan protocol sasl_ssl (memerlukan ssl dan sasl)', () => {
			const validDraft = {
				name: 'Kafka SASL SSL Sink',
				adm_status: 1,
				worker_count: 1,
				labels: '',
				batch: {
					batch_size: 1000,
					batch_timeout: 1000
				},
				channel: {
					type: 'memory',
					channel_buffer_size: 1000,
					non_blocking: false
				},
				config: {
					type: 'kafka',
					config: {
						hosts: ['kafka-broker:9095'],
						topic: 'secure-auth-events',
						format: 'json_otlp_batch',
						security: {
							protocol: 'sasl_ssl'
						},
						ssl: {
							insecure_skip_verify: true
						},
						sasl: {
							mechanism: 'sha512',
							username: 'admin',
							password: 'secretpassword'
						}
					}
				}
			}

			const res = AutoFormAdapter.toRequest(CreateSinkRequest, validDraft, sinkHints)
			expect(res.success).toBe(true)
			if (res.success) {
				const cfg = (res.data.config as any).config
				expect(cfg.security.protocol).toBe('sasl_ssl')
				expect(cfg.ssl).not.toBeNull()
				expect(cfg.ssl.insecure_skip_verify).toBe(true)
				expect(cfg.sasl.mechanism).toBe('sha512')
				expect(cfg.sasl.username).toBe('admin')
			}
		})

		it('Sink Form: auto-check Use SSL dan Use SASL secara reaktif saat protocol security berubah', async () => {
			const wrapper = mount(SinkForm, {
				global: {
					stubs: {
						'b-card': {
							template: '<div><slot name="header"/><slot/><slot name="footer"/></div>'
						},
						'b-icon': true,
						'b-heading': true,
						'b-flex': { template: '<div><slot/></div>' },
						'b-flex-item': { template: '<div><slot/></div>' },
						'b-button': true,
						'b-alert': true,
						'b-switch': true,
						BCheckbox: {
							name: 'BCheckbox',
							template:
								'<label><input type="checkbox" :checked="modelValue" @change="$emit(\'update:modelValue\', !modelValue)" /> {{ label }}</label>',
							props: ['modelValue', 'label', 'disabled']
						},
						'b-checkbox': {
							name: 'BCheckbox',
							template:
								'<label><input type="checkbox" :checked="modelValue" @change="$emit(\'update:modelValue\', !modelValue)" /> {{ label }}</label>',
							props: ['modelValue', 'label', 'disabled']
						},
						AppInputCluster: true,
						AppInputProcessor: true,
						AppInputText: true,
						AppInputLabel: true
					}
				}
			})

			await flushPromises()

			// 1. Change sink config type to 'kafka'
			const selects = wrapper.findAllComponents(AppInputSelect)
			const configTypeSelect = selects.find((s) => s.props('label') === 'Config Type')
			expect(configTypeSelect).toBeDefined()
			await configTypeSelect!.vm.$emit('update:modelValue', 'kafka')
			await flushPromises()

			// Find Security Protocol select
			const getProtocolSelect = () => {
				const currentSelects = wrapper.findAllComponents(AppInputSelect)
				return currentSelects.find((s) => s.props('label') === 'Security Protocol')
			}
			expect(getProtocolSelect()).toBeDefined()

			const findUnionCheckbox = (id: string) =>
				wrapper.findAllComponents({ name: 'Checkbox' }).find((c) => c.props('id') === id)

			// A. Test: protocol = 'ssl' -> auto-check Use SSL, sasl remains null
			await getProtocolSelect()!.vm.$emit('update:modelValue', 'ssl')
			await flushPromises()

			let sslCheckbox = findUnionCheckbox('union-check-config.config.ssl')
			let saslCheckbox = findUnionCheckbox('union-check-config.config.sasl')
			expect(sslCheckbox).toBeDefined()
			expect(sslCheckbox!.props('checked')).toBe(true)

			// B. Test: protocol = 'sasl_ssl' -> auto-check Use SSL and Use SASL
			await getProtocolSelect()!.vm.$emit('update:modelValue', 'sasl_ssl')
			await flushPromises()

			sslCheckbox = findUnionCheckbox('union-check-config.config.ssl')
			saslCheckbox = findUnionCheckbox('union-check-config.config.sasl')
			expect(sslCheckbox).toBeDefined()
			expect(sslCheckbox!.props('checked')).toBe(true)
			expect(saslCheckbox).toBeDefined()
			expect(saslCheckbox!.props('checked')).toBe(true)

			// C. Test: protocol = 'sasl_plaintext' -> auto-check Use SASL, ssl resets to false/null
			await getProtocolSelect()!.vm.$emit('update:modelValue', 'sasl_plaintext')
			await flushPromises()

			sslCheckbox = findUnionCheckbox('union-check-config.config.ssl')
			saslCheckbox = findUnionCheckbox('union-check-config.config.sasl')
			expect(saslCheckbox).toBeDefined()
			expect(saslCheckbox!.props('checked')).toBe(true)
			expect(sslCheckbox!.props('checked')).toBe(false)

			// D. Test: protocol = 'plaintext' -> both checkboxes return to unchecked / null
			await getProtocolSelect()!.vm.$emit('update:modelValue', 'plaintext')
			await flushPromises()

			sslCheckbox = findUnionCheckbox('union-check-config.config.ssl')
			saslCheckbox = findUnionCheckbox('union-check-config.config.sasl')
			expect(sslCheckbox!.props('checked')).toBe(false)
			expect(saslCheckbox!.props('checked')).toBe(false)
		})
	})

	describe('Listener Form: CreateListenerRequest with Netflow and Nested Address', () => {
		it('mengintrospeksi CreateListenerRequest dengan union Netflow dan UdpListener address', () => {
			const descriptors = introspectZodSchema(CreateListenerRequest, [], listenerHints)
			const configDesc = descriptors.find((d) => d.name === 'config')
			expect(configDesc).toBeDefined()
			expect(configDesc?.widget).toBe('discriminated-union')
			expect(configDesc?.unionDiscriminator).toBe('type')

			const netflowBranch = configDesc?.unionBranches?.['netflow']
			expect(netflowBranch).toBeDefined()

			const netflowConfig = netflowBranch?.find((d) => d.name === 'config')
			expect(netflowConfig).toBeDefined()

			const listenerChild = netflowConfig?.children?.find((d) => d.name === 'listener')
			expect(listenerChild).toBeDefined()

			const addressDesc = listenerChild?.children?.find((d) => d.name === 'address')
			expect(addressDesc).toBeDefined()
			expect(addressDesc?.pathKey).toBe('config.config.listener.address')
		})

		it('memunculkan validation error message pada nested address saat dikosongkan', () => {
			const invalidDraft = {
				name: 'My Netflow Listener',
				adm_status: 1,
				labels: '',
				config: {
					type: 'netflow',
					config: {
						batch_size: 1000,
						batch_timeout: 1000,
						listener: {
							address: ''
						}
					}
				}
			}

			const res = AutoFormAdapter.toRequest(
				CreateListenerRequest,
				invalidDraft,
				listenerHints
			)
			expect(res.success).toBe(false)
			if (!res.success) {
				// Canonical pathKey used by descriptor
				expect(res.issues['config.config.listener.address']).toBeDefined()
				expect(res.issues['config.config.listener.address']).toContain('required')

				// Alias path using branch discriminator name
				expect(res.issues['config.netflow.listener.address']).toBeDefined()
			}
		})

		it('berhasil memvalidasi draft saat nested address terisi dengan benar', () => {
			const validDraft = {
				name: 'Valid Netflow Listener',
				adm_status: 1,
				labels: '',
				config: {
					type: 'netflow',
					config: {
						batch_size: 1000,
						batch_timeout: 1000,
						listener: {
							address: '0.0.0.0:2055'
						}
					}
				}
			}

			const res = AutoFormAdapter.toRequest(CreateListenerRequest, validDraft, listenerHints)
			expect(res.success).toBe(true)
			if (res.success) {
				expect(res.data.config.type).toBe('netflow')
				expect((res.data.config as any).config.listener.address).toBe('0.0.0.0:2055')
				// Labels must not be undefined to prevent API missing field labels error
				expect(res.data.labels).toEqual({})
			}
		})

		it('memastikan AutoFormAdapter selalu menghasilkan objek valid untuk labels (bukan undefined/omitted) dan memparsing string json', () => {
			// 1. Draft with empty string labels
			const draftEmptyLabels = {
				name: 'Dummy Listener',
				adm_status: 1,
				labels: '',
				config: {
					type: 'dummy',
					config: {
						speed: 5,
						messages_per_batch: 100
					}
				}
			}
			const resEmpty = AutoFormAdapter.toRequest(
				CreateListenerRequest,
				draftEmptyLabels,
				listenerHints
			)
			expect(resEmpty.success).toBe(true)
			if (resEmpty.success) {
				expect(resEmpty.data.labels).toEqual({})
				// JSON stringify must not drop labels field
				const jsonStr = JSON.stringify(resEmpty.data)
				expect(jsonStr).toContain('"labels":{}')
			}

			// 2. Draft with JSON string labels
			const draftJsonLabels = {
				name: 'Dummy Listener 2',
				adm_status: 1,
				labels: JSON.stringify({ cluster: 'edge-1', tier: 'gateway' }),
				config: {
					type: 'dummy',
					config: {
						speed: 5,
						messages_per_batch: 100
					}
				}
			}
			const resJson = AutoFormAdapter.toRequest(
				CreateListenerRequest,
				draftJsonLabels,
				listenerHints
			)
			expect(resJson.success).toBe(true)
			if (resJson.success) {
				expect(resJson.data.labels).toEqual({ cluster: 'edge-1', tier: 'gateway' })
				const jsonStr = JSON.stringify(resJson.data)
				expect(jsonStr).toContain('"labels":{"cluster":"edge-1","tier":"gateway"}')
			}

			// 3. toDraft initializes labels to stringified JSON
			const initialDraft = AutoFormAdapter.toDraft(CreateListenerRequest, null, listenerHints)
			expect(initialDraft.labels).toBe('{}')

			// 4. toDraft converts backend labels object into stringified JSON for AppInputLabel
			const editDraft = AutoFormAdapter.toDraft(
				CreateListenerRequest,
				{
					name: 'Existing Listener',
					adm_status: 1,
					labels: { env: 'staging' }
				},
				listenerHints
			)
			expect(editDraft.labels).toBe('{"env":"staging"}')
		})

		it('mengintrospeksi CreateListenerRequest untuk branch snmp_trap dengan default values dan widgets yang tepat', () => {
			const descriptors = introspectZodSchema(CreateListenerRequest, [], listenerHints)
			const configDesc = descriptors.find((d) => d.name === 'config')
			const snmpTrapBranch = configDesc?.unionBranches?.['snmp_trap']
			expect(snmpTrapBranch).toBeDefined()

			const snmpTrapConfig = snmpTrapBranch?.find((d) => d.name === 'config')
			expect(snmpTrapConfig).toBeDefined()

			// Listener address
			const listenerDesc = snmpTrapConfig?.children?.find((c) => c.name === 'listener')
			const addressDesc = listenerDesc?.children?.find((c) => c.name === 'address')
			expect(addressDesc?.defaultValue).toBe('0.0.0.0:162')

			// v2c communities
			const v2cDesc = snmpTrapConfig?.children?.find((c) => c.name === 'v2c')
			const v2cCommunitiesDesc = v2cDesc?.children?.find((c) => c.name === 'communities')
			expect(v2cCommunitiesDesc?.widget).toBe('array-string')
			expect(v2cCommunitiesDesc?.defaultValue).toEqual(['public'])

			// v3 security models
			const v3Desc = snmpTrapConfig?.children?.find((c) => c.name === 'v3')
			const secModelsDesc = v3Desc?.children?.find((c) => c.name === 'security_models')
			expect(secModelsDesc?.widget).toBe('array-object')

			// Check password widgets inside security_models
			const secConfigDesc = secModelsDesc?.children?.find((c) => c.name === 'config')
			const authDesc = secConfigDesc?.children?.find((c) => c.name === 'authentication')
			const authPassDesc = authDesc?.children?.find((c) => c.name === 'passphrase')
			expect(authPassDesc?.widget).toBe('password')

			const privDesc = secConfigDesc?.children?.find((c) => c.name === 'privacy')
			const privPassDesc = privDesc?.children?.find((c) => c.name === 'passphrase')
			expect(privPassDesc?.widget).toBe('password')
		})

		it('AutoFormAdapter.toDraft menghasilkan default SNMP Trap dengan address dan v2c community', () => {
			const draft = AutoFormAdapter.toDraft(
				CreateListenerRequest,
				{
					name: 'SNMP Trap Draft',
					config: {
						type: 'snmp_trap'
					}
				},
				listenerHints
			)

			expect(draft.config.type).toBe('snmp_trap')
			expect(draft.config.config.listener.address).toBe('0.0.0.0:162')
			expect(draft.batch.batch_size).toBe(1000)
			expect(draft.batch.batch_timeout).toBe(1000)
			expect(draft.config.config.v2c.communities).toEqual(['public'])
		})

		it('AutoFormAdapter.toRequest memvalidasi dan mensanitasi form SNMP Trap dengan v2c community', () => {
			const draft = {
				name: 'Production Trap Listener',
				adm_status: 1,
				worker_count: 1,
				disable_cluster_forwarding: true,
				labels: '',
				config: {
					type: 'snmp_trap',
					config: {
						batch_size: 1000,
						batch_timeout: 1000,
						listener: {
							address: '0.0.0.0:162'
						},
						v1: { communities: [] },
						v2c: { communities: ['public', 'internal'] },
						v3: { security_models: [] }
					}
				}
			}

			const res = AutoFormAdapter.toRequest(CreateListenerRequest, draft, listenerHints)
			expect(res.success).toBe(true)
			if (res.success) {
				const cfg = (res.data.config as any).config
				expect(cfg.listener.address).toBe('0.0.0.0:162')
				expect(cfg.v2c.communities).toEqual(['public', 'internal'])
				// Empty communities dan empty security_models disanitasi menjadi null
				expect(cfg.v1).toBeNull()
				expect(cfg.v3).toBeNull()
			}
		})

		it('AutoFormAdapter.toRequest memvalidasi SNMP Trap dengan v3 USM security model', () => {
			const draft = {
				name: 'SNMP v3 Trap Listener',
				adm_status: 1,
				worker_count: 1,
				disable_cluster_forwarding: true,
				labels: '',
				config: {
					type: 'snmp_trap',
					config: {
						batch_size: 1000,
						batch_timeout: 1000,
						listener: {
							address: '0.0.0.0:162'
						},
						v1: { communities: [] },
						v2c: { communities: [] },
						v3: {
							security_models: [
								{
									type: 'usm',
									config: {
										username: 'trapadmin',
										authentication: {
											protocol: 'sha256',
											passphrase: 'authpassword123'
										},
										privacy: {
											protocol: 'aes128',
											passphrase: 'privpassword123'
										}
									}
								}
							]
						}
					}
				}
			}

			const res = AutoFormAdapter.toRequest(CreateListenerRequest, draft, listenerHints)
			expect(res.success).toBe(true)
			if (res.success) {
				const cfg = (res.data.config as any).config
				expect(cfg.v1).toBeNull()
				expect(cfg.v2c).toBeNull()
				expect(cfg.v3.security_models).toHaveLength(1)
				expect(cfg.v3.security_models[0].type).toBe('usm')
				expect(cfg.v3.security_models[0].config.username).toBe('trapadmin')
				expect(cfg.v3.security_models[0].config.authentication.passphrase).toBe(
					'authpassword123'
				)
				expect(cfg.v3.security_models[0].config.privacy.passphrase).toBe('privpassword123')
			}
		})

		it('AutoFormAdapter.toRequest menolak SNMP Trap jika v1, v2c, dan v3 semuanya kosong', () => {
			const draft = {
				name: 'Empty Trap Listener',
				adm_status: 1,
				labels: '',
				config: {
					type: 'snmp_trap',
					config: {
						batch_size: 1000,
						batch_timeout: 1000,
						listener: {
							address: '0.0.0.0:162'
						},
						v1: { communities: [] },
						v2c: { communities: [] },
						v3: { security_models: [] }
					}
				}
			}

			const res = AutoFormAdapter.toRequest(CreateListenerRequest, draft, listenerHints)
			expect(res.success).toBe(false)
			if (!res.success) {
				expect(res.issues['config.config.v2c.communities']).toContain(
					'At least one SNMP version'
				)
			}
		})

		it('AutoFormAdapter.toRequest menolak SNMP v3 jika privacy dikonfigurasi tanpa authentication', () => {
			const draft = {
				name: 'Invalid v3 Trap Listener',
				adm_status: 1,
				labels: '',
				config: {
					type: 'snmp_trap',
					config: {
						batch_size: 1000,
						batch_timeout: 1000,
						listener: {
							address: '0.0.0.0:162'
						},
						v3: {
							security_models: [
								{
									type: 'usm',
									config: {
										username: 'trapadmin',
										authentication: null,
										privacy: {
											protocol: 'aes128',
											passphrase: 'privpassword123'
										}
									}
								}
							]
						}
					}
				}
			}

			const res = AutoFormAdapter.toRequest(CreateListenerRequest, draft, listenerHints)
			expect(res.success).toBe(false)
			if (!res.success) {
				expect(
					res.issues[
						'config.config.v3.security_models.0.config.authentication.passphrase'
					]
				).toContain('Authentication is required')
			}
		})

		it('mengintrospeksi CreateListenerRequest untuk branch open_telemetry dengan konfigurasi mode, batch, listener, dan tls', () => {
			const descriptors = introspectZodSchema(CreateListenerRequest, [], listenerHints)
			const configDesc = descriptors.find((d) => d.name === 'config')
			const otelBranch = configDesc?.unionBranches?.['open_telemetry']
			expect(otelBranch).toBeDefined()

			const otelConfig = otelBranch?.find((d) => d.name === 'config')
			expect(otelConfig).toBeDefined()

			// mode
			const modeDesc = otelConfig?.children?.find((c) => c.name === 'mode')
			expect(modeDesc?.label).toBe('Protocol Mode')
			expect(modeDesc?.defaultValue).toBe('otap')
			expect(modeDesc?.options?.map((o) => o.value)).toEqual(['otap', 'otlp', 'both'])

			// batch
			const batchDesc = descriptors.find((d) => d.name === 'batch')
			const batchSizeDesc = batchDesc?.children?.find((c) => c.name === 'batch_size')
			expect(batchSizeDesc?.label).toBe('Batch Size')
			expect(batchSizeDesc?.defaultValue).toBe(1000)

			// listener
			const listenerDesc = otelConfig?.children?.find((c) => c.name === 'listener')
			expect(listenerDesc?.children).toBeDefined()
			const addressDesc = listenerDesc?.children?.find((c) => c.name === 'address')
			expect(addressDesc?.label).toBe('Listener Address')
			expect(addressDesc?.defaultValue).toBe('0.0.0.0:4317')

			const whitelistDesc = listenerDesc?.children?.find((c) => c.name === 'ip_whitelist')
			expect(whitelistDesc?.widget).toBe('array-string')

			// tls
			const tlsDesc = otelConfig?.children?.find((c) => c.name === 'tls')
			expect(tlsDesc).toBeDefined()
			expect(tlsDesc?.hints?.group).toBe('TLS Encryption')
			// Ensure tls is not hidden by visibleWhen
			expect(tlsDesc?.hints?.visibleWhen).toBeUndefined()
		})

		it('AutoFormAdapter.toDraft menghasilkan default OpenTelemetry dengan mode, address, dan tls null', () => {
			const draft = AutoFormAdapter.toDraft(
				CreateListenerRequest,
				{
					name: 'OTel Listener Draft',
					config: {
						type: 'open_telemetry'
					}
				},
				listenerHints
			)

			expect(draft.config.type).toBe('open_telemetry')
			expect(draft.config.config.mode).toBe('otap')
			expect(draft.batch.batch_size).toBe(1000)
			expect(draft.batch.batch_timeout).toBe(1000)
			expect(draft.config.config.listener.address).toBe('0.0.0.0:4317')
			expect(draft.config.config.tls).toBeNull()
		})

		it('AutoFormAdapter.toRequest memvalidasi dan mensanitasi OpenTelemetry tanpa TLS (plain TCP)', () => {
			const draft = {
				name: 'Production OTel Listener',
				adm_status: 1,
				worker_count: 2,
				disable_cluster_forwarding: false,
				labels: '',
				batch: {
					batch_size: 1000,
					batch_timeout: 1000
				},
				config: {
					type: 'open_telemetry',
					config: {
						mode: 'otap',
						listener: {
							address: '0.0.0.0:4317',
							ip_whitelist: [],
							ip_blacklist: []
						},
						tls: {
							cert_path: '',
							key_path: '',
							ca_cert_path: '',
							key_password: '',
							insecure_skip_verify: false
						}
					}
				}
			}

			const res = AutoFormAdapter.toRequest(CreateListenerRequest, draft, listenerHints)
			expect(res.success).toBe(true)
			if (res.success) {
				const cfg = (res.data.config as any).config
				expect(cfg.mode).toBe('otap')
				expect(res.data.batch?.batch_size).toBe(1000)
				expect(cfg.listener.address).toBe('0.0.0.0:4317')
				// Empty tls is sanitized to null
				expect(cfg.tls).toBeNull()
			}
		})

		it('AutoFormAdapter.toRequest memvalidasi OpenTelemetry dengan TLS lengkap', () => {
			const draft = {
				name: 'Secure OTel Listener',
				adm_status: 1,
				labels: '',
				config: {
					type: 'open_telemetry',
					config: {
						mode: 'both',
						batch_size: 2000,
						batch_timeout: 500,
						listener: {
							address: '0.0.0.0:4317'
						},
						tls: {
							cert_path: '/etc/ssl/certs/server.crt',
							key_path: '/etc/ssl/private/server.key',
							ca_cert_path: '/etc/ssl/certs/ca.crt',
							insecure_skip_verify: true
						}
					}
				}
			}

			const res = AutoFormAdapter.toRequest(CreateListenerRequest, draft, listenerHints)
			expect(res.success).toBe(true)
			if (res.success) {
				const cfg = (res.data.config as any).config
				expect(cfg.mode).toBe('both')
				expect(cfg.tls).not.toBeNull()
				expect(cfg.tls.cert_path).toBe('/etc/ssl/certs/server.crt')
				expect(cfg.tls.key_path).toBe('/etc/ssl/private/server.key')
				expect(cfg.tls.ca_cert_path).toBe('/etc/ssl/certs/ca.crt')
				expect(cfg.tls.insecure_skip_verify).toBe(true)
			}
		})

		it('AutoFormAdapter.toRequest menolak OpenTelemetry jika cert_path diisi tanpa key_path', () => {
			const draft = {
				name: 'Invalid TLS OTel Listener',
				adm_status: 1,
				labels: '',
				config: {
					type: 'open_telemetry',
					config: {
						mode: 'otap',
						batch_size: 1000,
						batch_timeout: 1000,
						listener: {
							address: '0.0.0.0:4317'
						},
						tls: {
							cert_path: '/etc/ssl/certs/server.crt',
							key_path: ''
						}
					}
				}
			}

			const res = AutoFormAdapter.toRequest(CreateListenerRequest, draft, listenerHints)
			expect(res.success).toBe(false)
			if (!res.success) {
				expect(res.issues['config.config.tls.key_path']).toContain(
					'Private key path is required'
				)
			}
		})

		it('menginisialisasi draft listener dengan port default yang tepat untuk masing-masing listener type', () => {
			const snmpDraft = AutoFormAdapter.toDraft(
				CreateListenerRequest,
				{ config: { type: 'snmp_trap' } },
				listenerHints
			)
			expect(snmpDraft.config.config.listener.address).toBe('0.0.0.0:162')

			const netflowDraft = AutoFormAdapter.toDraft(
				CreateListenerRequest,
				{ config: { type: 'netflow' } },
				listenerHints
			)
			expect(netflowDraft.config.config.listener.address).toBe('0.0.0.0:2055')

			const syslogDraft = AutoFormAdapter.toDraft(
				CreateListenerRequest,
				{ config: { type: 'syslog' } },
				listenerHints
			)
			expect(syslogDraft.config.config.listener.udp.address).toBe('0.0.0.0:514')
		})

		it('tidak memetakan nested error worker_count ke root worker_count field', () => {
			const rootErr = AutoFormAdapter.validateField(
				CreateListenerRequest,
				'worker_count',
				{ name: 'Test', config: { type: 'dummy', config: {} } },
				listenerHints
			)
			expect(rootErr).toBeNull()
		})
	})
})

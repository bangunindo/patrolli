// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import '@/prototypes/array.prototype'
import { describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import ListenerForm from '@/components/configuration/listener/Form.vue'
import { useListenerStore } from '@/stores/listeners.store'
import { FormMode } from '@@/shared/types/app.type'

import { convertZodToJsonSchema } from '@/app/services/forms/json-schema-converter'
import { CreateListenerRequest } from '@@/shared/generated/patrolliAPI.schemas.type'

describe('Listener DynamicZodForm Integration', () => {
	const commonStubs = {
		'b-card': {
			template: '<div><slot name="header"/><slot/><slot name="footer"/></div>'
		},
		'b-icon': true,
		'b-heading': true,
		'b-flex': { template: '<div><slot/></div>' },
		'b-flex-item': { template: '<div><slot/></div>' },
		'b-button': {
			template: '<button type="button"><slot/></button>'
		},
		AppInputCluster: true,
		AppInputText: true,
		AppInputLabel: true
	}

	it('should render configurations for listener types dummy, snmp_trap, syslog, netflow, and open_telemetry', () => {
		const json = convertZodToJsonSchema(CreateListenerRequest.shape.config) as any
		const branches = json.anyOf || []

		const dummyBranch = branches.find((b: any) => b.properties?.type?.enum?.[0] === 'dummy')
		expect(dummyBranch).toBeDefined()
		expect(dummyBranch.properties.config.properties).toHaveProperty('messages_per_batch')
		expect(dummyBranch.properties.config.properties).toHaveProperty('speed')

		const snmpBranch = branches.find((b: any) => b.properties?.type?.enum?.[0] === 'snmp_trap')
		expect(snmpBranch).toBeDefined()
		expect(snmpBranch.properties.config.properties).toHaveProperty('listener')
		expect(snmpBranch.properties.config.properties).toHaveProperty('v2c')

		const syslogBranch = branches.find((b: any) => b.properties?.type?.enum?.[0] === 'syslog')
		expect(syslogBranch).toBeDefined()
		expect(syslogBranch.properties.config.properties).toHaveProperty('listener')

		const netflowBranch = branches.find((b: any) => b.properties?.type?.enum?.[0] === 'netflow')
		expect(netflowBranch).toBeDefined()
		expect(netflowBranch.properties.config.properties).toHaveProperty('listener')

		const otelBranch = branches.find(
			(b: any) => b.properties?.type?.enum?.[0] === 'open_telemetry'
		)
		expect(otelBranch).toBeDefined()
		expect(otelBranch.properties.config.properties).toHaveProperty('listener')
	})

	it('should render ListenerForm with DynamicZodForm in ADD mode', async () => {
		const pinia = createPinia()
		setActivePinia(pinia)
		const store = useListenerStore()
		store.listenerFormMode = FormMode.ADD

		const wrapper = mount(ListenerForm, {
			global: {
				plugins: [pinia],
				stubs: commonStubs
			}
		})

		await flushPromises()

		const vm = wrapper.vm as any
		expect(vm.form.adm_status).toBe(1)
		expect(vm.form.worker_count).toBe(1)
		expect(vm.form.disable_cluster_forwarding).toBe(true)
		expect(wrapper.findComponent({ name: 'DynamicZodForm' }).exists()).toBe(true)
	})

	it('should render ListenerForm with Syslog configuration in UPDATE mode and submit successfully', async () => {
		const pinia = createPinia()
		setActivePinia(pinia)
		const store = useListenerStore()
		store.listenerFormMode = FormMode.UPDATE
		store.currentListener = {
			listener_id: 'listener-syslog-1',
			name: 'Syslog UDP Production',
			adm_status: 1,
			worker_count: 2,
			disable_cluster_forwarding: false,
			labels: { env: 'prod', app: 'syslog' },
			cluster_name: 'Test-Cluster',
			cluster_url: 'http://test-cluster:8080',
			config: {
				type: 'syslog',
				config: {
					batch_size: 1000,
					batch_timeout: 1000,
					listener: {
						udp: {
							address: '0.0.0.0:514'
						}
					}
				}
			}
		} as any

		let submittedPayload: any = null
		store.updateListener = vi.fn().mockImplementation((id, payload, clusters) => {
			submittedPayload = { id, payload, clusters }
			return Promise.resolve({ success: true })
		})

		const wrapper = mount(ListenerForm, {
			global: {
				plugins: [pinia],
				stubs: commonStubs
			}
		})

		await flushPromises()

		const vm = wrapper.vm as any
		expect(vm.form.name).toBe('Syslog UDP Production')
		expect(vm.form.worker_count).toBe(2)
		expect(vm.form.disable_cluster_forwarding).toBe(false)
		expect(vm.form.config.type).toBe('syslog')
		expect(vm.form.config.config.listener.udp.address).toBe('0.0.0.0:514')

		// Trigger handleFormSubmit
		await vm.handleFormSubmit(vm.form)
		await flushPromises()

		expect(store.updateListener).toHaveBeenCalled()
		expect(submittedPayload.id).toBe('listener-syslog-1')
		expect(submittedPayload.payload.name).toBe('Syslog UDP Production')
		expect(submittedPayload.payload.labels).toEqual({ env: 'prod', app: 'syslog' })
		expect(submittedPayload.payload.config.type).toBe('syslog')
		expect(submittedPayload.payload.config.config.listener.udp.address).toBe('0.0.0.0:514')
	})

	it('should render ListenerForm with SNMP Trap configuration in UPDATE mode and validate payload', async () => {
		const pinia = createPinia()
		setActivePinia(pinia)
		const store = useListenerStore()
		store.listenerFormMode = FormMode.UPDATE
		store.currentListener = {
			listener_id: 'listener-snmp-1',
			name: 'SNMP Trap Listener',
			adm_status: 1,
			worker_count: 1,
			disable_cluster_forwarding: true,
			labels: '{}',
			config: {
				type: 'snmp_trap',
				config: {
					batch_size: 1000,
					batch_timeout: '500', // numeric string to test parseInt
					listener: {
						address: '0.0.0.0:162'
					},
					v2c: {
						communities: ['public']
					}
				}
			}
		} as any

		let submittedPayload: any = null
		store.updateListener = vi.fn().mockImplementation((id, payload, clusters) => {
			submittedPayload = { id, payload, clusters }
			return Promise.resolve({ success: true })
		})

		const wrapper = mount(ListenerForm, {
			global: {
				plugins: [pinia],
				stubs: commonStubs
			}
		})

		await flushPromises()

		const vm = wrapper.vm as any
		expect(vm.form.name).toBe('SNMP Trap Listener')
		expect(vm.form.config.type).toBe('snmp_trap')

		await vm.handleFormSubmit(vm.form)
		await flushPromises()

		expect(store.updateListener).toHaveBeenCalled()
		expect(submittedPayload.payload.config.config.batch_timeout).toBe(500)
		expect(submittedPayload.payload.config.config.v2c.communities).toEqual(['public'])
		expect(submittedPayload.payload.labels).toEqual({})
	})

	it('should successfully validate and submit dummy listener in ADD mode', async () => {
		const pinia = createPinia()
		setActivePinia(pinia)
		const store = useListenerStore()
		store.listenerFormMode = FormMode.ADD

		let createdPayload: any = null
		store.createListener = vi.fn().mockImplementation((payload, clusters) => {
			createdPayload = { payload, clusters }
			return Promise.resolve({ success: true })
		})

		const wrapper = mount(ListenerForm, {
			global: {
				plugins: [pinia],
				stubs: commonStubs
			}
		})

		await flushPromises()

		const vm = wrapper.vm as any
		// Populate mandatory 'name' field
		vm.form.name = 'Test Dummy Listener'
		expect(vm.form.config.type).toBe('dummy')
		expect(vm.form.config.config).toEqual({
			messages_per_batch: 100,
			speed: 5
		})

		// Trigger dynamic form submission
		await vm.triggerSubmit()
		await flushPromises()

		expect(store.createListener).toHaveBeenCalled()
		expect(createdPayload.payload.name).toBe('Test Dummy Listener')
		expect(createdPayload.payload.config.type).toBe('dummy')
		expect(createdPayload.payload.config.config).toEqual({
			messages_per_batch: 100,
			speed: 5
		})
	})

	it('should transform dummy listener string inputs to integers correctly', async () => {
		const pinia = createPinia()
		setActivePinia(pinia)
		const store = useListenerStore()
		store.listenerFormMode = FormMode.ADD

		let createdPayload: any = null
		store.createListener = vi.fn().mockImplementation((payload, clusters) => {
			createdPayload = { payload, clusters }
			return Promise.resolve({ success: true })
		})

		const wrapper = mount(ListenerForm, {
			global: {
				plugins: [pinia],
				stubs: commonStubs
			}
		})

		await flushPromises()

		const vm = wrapper.vm as any
		vm.form.name = 'Custom Dummy Listener'
		vm.form.config = {
			type: 'dummy',
			config: {
				messages_per_batch: '250',
				speed: '10'
			}
		}

		await vm.triggerSubmit()
		await flushPromises()

		expect(store.createListener).toHaveBeenCalled()
		expect(createdPayload.payload.config.config).toEqual({
			messages_per_batch: 250,
			speed: 10
		})
	})

	it('should keep root name isolated from internal_logs include[].name and avoid duplicating AppInputCluster', async () => {
		const pinia = createPinia()
		setActivePinia(pinia)
		const store = useListenerStore()
		store.listenerFormMode = FormMode.UPDATE
		store.currentListener = {
			listener_id: 'listener-internal-logs-1',
			name: 'Internal Logs Listener',
			adm_status: 1,
			worker_count: 1,
			disable_cluster_forwarding: true,
			labels: {},
			cluster_name: 'Test-Cluster',
			cluster_url: 'http://test-cluster:8080',
			config: {
				type: 'internal_logs',
				config: {
					buffer_size: 10000,
					level: 'info',
					include: [{ name: 'patrolli::pipeline', level: 'debug' }],
					exclude: ['patrolli::noisy']
				}
			}
		} as any

		let submittedPayload: any = null
		store.updateListener = vi.fn().mockImplementation((id, payload, clusters) => {
			submittedPayload = { id, payload, clusters }
			return Promise.resolve({ success: true })
		})

		const wrapper = mount(ListenerForm, {
			global: {
				plugins: [pinia],
				stubs: {
					...commonStubs,
					AppInputCluster: {
						template: '<div data-testid="cluster-input" />'
					},
					AppInputText: {
						props: ['modelValue', 'label', 'placeholder'],
						emits: ['update:modelValue'],
						template:
							'<input data-testid="text-input" :data-label="label" :placeholder="placeholder" :value="modelValue" @input="$emit(\'update:modelValue\', $event.target.value)" />'
					}
				}
			}
		})

		await flushPromises()

		expect(wrapper.findAll('[data-testid="cluster-input"]')).toHaveLength(1)

		const moduleNameInput = wrapper.find('[data-testid="text-input"][data-label="Module Name"]')
		expect(moduleNameInput.exists()).toBe(true)
		expect((moduleNameInput.element as HTMLInputElement).value).toBe('patrolli::pipeline')
		expect(moduleNameInput.attributes('placeholder')).toBe('e.g. patrolli::pipeline')

		await moduleNameInput.setValue('patrolli::collector')
		await flushPromises()

		const vm = wrapper.vm as any
		expect(vm.form.name).toBe('Internal Logs Listener')
		expect(vm.form.config.config.include[0].name).toBe('patrolli::collector')

		await vm.triggerSubmit()
		await flushPromises()

		expect(store.updateListener).toHaveBeenCalled()
		expect(submittedPayload.payload.name).toBe('Internal Logs Listener')
		expect(submittedPayload.payload.config.config.include).toEqual([
			{ name: 'patrolli::collector', level: 'debug' }
		])
	})
})

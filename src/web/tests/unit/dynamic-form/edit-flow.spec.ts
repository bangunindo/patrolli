// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import '@@/shared/types/array.type'
import { describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import EndpointForm from '@/components/configuration/endpoint/Form.vue'
import ListenerForm from '@/components/configuration/listener/Form.vue'
import SinkForm from '@/components/configuration/sinks/Form.vue'
import { useEndpointsStore } from '@/stores/endpoints.store'
import { useListenerStore } from '@/stores/listeners.store'
import { useSinksStore } from '@/stores/sinks.store'
import { FormMode } from '@@/shared/types/app.type'

describe('Edit Flow Tests for Endpoints, Listeners, Processors, and Sinks', () => {
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
		'b-alert': true,
		AppInputCluster: true,
		AppInputHost: true,
		AppInputItem: true,
		AppInputProcessor: true,
		AppInputLabel: true
	}

	it('EndpointForm loads existing endpoint and does not wipe host_id on mount in UPDATE mode', async () => {
		const pinia = createPinia()
		setActivePinia(pinia)
		const store = useEndpointsStore()

		store.endpointFormMode = FormMode.UPDATE
		store.currentEndpoint = {
			endpoint_id: 'ep-123',
			name: 'Test Endpoint Router',
			host_id: 'host-xyz-456',
			adm_status: 1,
			cluster_name: 'Cluster-Beta',
			cluster_url: 'http://cluster-beta.local:8080',
			config: {
				type: 'snmp_v2c',
				config: {
					address: '192.168.1.1:161',
					community: 'custom-secret'
				}
			}
		} as any

		const wrapper = mount(EndpointForm, {
			global: {
				plugins: [pinia],
				stubs: commonStubs
			}
		})

		await flushPromises()

		// Verify that host_id is preserved and not cleared by selectedCluster watcher
		const vm = wrapper.vm as any
		expect(vm.form.name).toBe('Test Endpoint Router')
		expect(vm.form.host_id).toBe('host-xyz-456')
		expect(vm.form.adm_status).toBe(1)
		expect(vm.form.config.type).toBe('snmp_v2c')
		expect(vm.form.config.config.address).toBe('192.168.1.1:161')
		expect(vm.form.config.config.community).toBe('custom-secret')

		// Test updating and submitting in UPDATE mode
		let updatedPayload: any = null
		store.updateEndpoint = vi.fn().mockImplementation((id, payload, clusters) => {
			updatedPayload = { id, payload, clusters }
			return Promise.resolve({ success: true })
		})

		vm.form.name = 'Updated Endpoint Router'
		await vm.handleFormSubmit(vm.form)
		await flushPromises()

		expect(store.updateEndpoint).toHaveBeenCalled()
		expect(updatedPayload.id).toBe('ep-123')
		expect(updatedPayload.payload.name).toBe('Updated Endpoint Router')
		expect(updatedPayload.payload.host_id).toBe('host-xyz-456')
		expect(updatedPayload.payload.config.type).toBe('snmp_v2c')
	})

	it('ListenerForm loads existing listener in UPDATE mode and preserves all configurations', async () => {
		const pinia = createPinia()
		setActivePinia(pinia)
		const store = useListenerStore()

		store.listenerFormMode = FormMode.UPDATE
		store.currentListener = {
			listener_id: 'lis-999',
			name: 'Core Syslog Listener',
			adm_status: 1,
			worker_count: 4,
			disable_cluster_forwarding: false,
			cluster_name: 'Cluster-Omega',
			cluster_url: 'http://cluster-omega.local:8080',
			labels: { env: 'production', role: 'ingest' },
			config: {
				type: 'syslog',
				config: {
					address: '0.0.0.0:514',
					protocol: 'udp'
				}
			}
		} as any

		const wrapper = mount(ListenerForm, {
			global: {
				plugins: [pinia],
				stubs: commonStubs
			}
		})

		await flushPromises()

		const vm = wrapper.vm as any
		expect(vm.form.name).toBe('Core Syslog Listener')
		expect(vm.form.worker_count).toBe(4)
		expect(vm.form.disable_cluster_forwarding).toBe(false)
		expect(vm.form.config.type).toBe('syslog')
		expect(vm.form.config.config.address).toBe('0.0.0.0:514')
		expect(vm.form.config.config.protocol).toBe('udp')
		expect(JSON.parse(vm.form.labels)).toEqual({ env: 'production', role: 'ingest' })

		// Test updating and submitting in UPDATE mode
		let updatedPayload: any = null
		store.updateListener = vi.fn().mockImplementation((id, payload, clusters) => {
			updatedPayload = { id, payload, clusters }
			return Promise.resolve({ success: true })
		})

		vm.form.worker_count = 8
		await vm.handleFormSubmit(vm.form)
		await flushPromises()

		expect(store.updateListener).toHaveBeenCalled()
		expect(updatedPayload.id).toBe('lis-999')
		expect(updatedPayload.payload.worker_count).toBe(8)
		expect(updatedPayload.payload.config.type).toBe('syslog')
		expect(updatedPayload.payload.labels).toEqual({ env: 'production', role: 'ingest' })
	})

	it('SinkForm loads existing sink in UPDATE mode and does not wipe inputs', async () => {
		const pinia = createPinia()
		setActivePinia(pinia)
		const store = useSinksStore()

		store.sinkFormMode = FormMode.UPDATE
		store.currentSink = {
			sink_id: 'sink-777',
			name: 'Kafka Export Sink',
			adm_status: 1,
			worker_count: 2,
			cluster_name: 'Cluster-Delta',
			cluster_url: 'http://cluster-delta.local:8080',
			inputs: ['pipeline-output-1', 'pipeline-output-2'],
			batch: {
				batch_size: 500,
				batch_timeout: 2000
			},
			channel: {
				type: 'memory',
				compression: 'snappy'
			},
			config: {
				type: 'kafka',
				config: {
					brokers: ['kafka.cluster.local:9092'],
					topic: 'telemetry_out'
				}
			}
		} as any

		const wrapper = mount(SinkForm, {
			props: {
				formInputs: ['pipeline-output-1', 'pipeline-output-2']
			},
			global: {
				plugins: [pinia],
				stubs: commonStubs
			}
		})

		await flushPromises()

		const vm = wrapper.vm as any
		expect(vm.form.name).toBe('Kafka Export Sink')
		expect(vm.form.inputs).toEqual(['pipeline-output-1', 'pipeline-output-2'])
		expect(vm.form.worker_count).toBe(2)
		expect(vm.form.batch.batch_size).toBe(500)
		expect(vm.form.batch.batch_timeout).toBe(2000)
		expect(vm.form.config.type).toBe('kafka')

		// Test updating and submitting in UPDATE mode
		let updatedPayload: any = null
		store.updateSink = vi.fn().mockImplementation((id, payload, clusters) => {
			updatedPayload = { id, payload, clusters }
			return Promise.resolve({ success: true })
		})

		vm.form.name = 'Updated Kafka Export Sink'
		await vm.handleFormSubmit(vm.form)
		await flushPromises()

		expect(store.updateSink).toHaveBeenCalled()
		expect(updatedPayload.id).toBe('sink-777')
		expect(updatedPayload.payload.name).toBe('Updated Kafka Export Sink')
		expect(updatedPayload.payload.inputs).toEqual(['pipeline-output-1', 'pipeline-output-2'])
		expect(updatedPayload.payload.config.type).toBe('kafka')
	})

	it('Listener ModalForm correctly preserves and populates name when switching from ADD to UPDATE', async () => {
		const pinia = createPinia()
		setActivePinia(pinia)
		const store = useListenerStore()

		const testListener = {
			listener_id: 'lis-999',
			name: 'My Populated Syslog Listener',
			adm_status: 1,
			worker_count: 4,
			disable_cluster_forwarding: false,
			cluster_name: 'Cluster-Omega',
			cluster_url: 'http://cluster-omega.local:8080',
			labels: { env: 'production' },
			config: {
				type: 'syslog',
				config: {
					address: '0.0.0.0:514',
					protocol: 'udp'
				}
			}
		}

		// Initial state: User previously opened ADD modal
		store.toggleListenerModalForm(true, FormMode.ADD, null)

		const ModalListenerForm = (
			await import('@/components/configuration/listener/ModalForm.vue')
		).default
		const wrapper = mount(ModalListenerForm, {
			global: {
				plugins: [pinia],
				stubs: {
					...commonStubs,
					'b-modal': {
						props: ['modelValue'],
						template: '<div v-show="modelValue" class="b-modal-stub"><slot/></div>'
					}
				}
			}
		})

		await flushPromises()
		const formComp = wrapper.findComponent(ListenerForm)
		expect((formComp.vm as any).form.name).toBe('')

		// User cancels/closes modal
		store.toggleListenerModalForm(false)
		await flushPromises()

		// User clicks edit on an existing listener row
		store.toggleListenerModalForm(true, FormMode.UPDATE, testListener as any)
		await flushPromises()

		expect((formComp.vm as any).form.name).toBe('My Populated Syslog Listener')
	}, 60000)
})

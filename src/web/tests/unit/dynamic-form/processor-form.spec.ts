// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import '@@/shared/types/array.type'
import { describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import ProcessorForm from '@/components/configuration/processor/Form.vue'
import { useProcessorStore } from '@/stores/processor.store'
import { FormMode } from '@@/shared/types/app.type'
import { AutoFormAdapter } from '@/app/services/forms/auto-adapter'
import { AddAttributes, CreateProcessorRequest } from '@@/shared/generated/patrolliAPI.schemas.type'
import { processorHints } from '@/app/services/forms/ui-hints/processor.hints'

describe('Processor DynamicZodForm Integration', () => {
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
		AppInputItem: true,
		AppInputProcessor: true,
		AppInputText: true
	}

	it('merender ProcessorForm dengan DynamicZodForm pada mode ADD', async () => {
		const pinia = createPinia()
		setActivePinia(pinia)
		const store = useProcessorStore()
		store.processorFormMode = FormMode.ADD

		const wrapper = mount(ProcessorForm, {
			global: {
				plugins: [pinia],
				stubs: commonStubs
			}
		})

		await flushPromises()

		const vm = wrapper.vm as any
		expect(vm.form.adm_status).toBe(1)
		expect(vm.form.config.module).toBe('null')
		expect(wrapper.findComponent({ name: 'DynamicZodForm' }).exists()).toBe(true)
	})

	it('merender ProcessorForm dengan modul drop_attributes pada mode UPDATE dan submit berhasil', async () => {
		const pinia = createPinia()
		setActivePinia(pinia)
		const store = useProcessorStore()
		store.processorFormMode = FormMode.UPDATE
		store.currentProcessor = {
			processor_id: 'proc-test-1',
			name: 'Drop Meta Attributes',
			adm_status: 1,
			cluster_name: 'Test-Cluster',
			cluster_url: 'http://test-cluster:8080',
			input_items: ['item-uuid-1'],
			input_processors: ['pipe-uuid-1'],
			config: {
				module: 'drop_attributes',
				config: {
					keys: {
						record: ['debug.info']
					}
				}
			}
		} as any

		let submittedPayload: any = null
		store.updateProcessor = vi.fn().mockImplementation((id, payload, clusters) => {
			submittedPayload = { id, payload, clusters }
			return Promise.resolve({ success: true })
		})

		const wrapper = mount(ProcessorForm, {
			global: {
				plugins: [pinia],
				stubs: commonStubs
			}
		})

		await flushPromises()

		const vm = wrapper.vm as any
		expect(vm.form.name).toBe('Drop Meta Attributes')
		expect(vm.form.input_items).toEqual(['item-uuid-1'])
		expect(vm.form.input_processors).toEqual(['pipe-uuid-1'])
		expect(vm.form.config.module).toBe('drop_attributes')
		expect(vm.form.config.config.keys).toEqual({ record: ['debug.info'] })

		// Trigger handleFormSubmit
		await vm.handleFormSubmit(vm.form)
		await flushPromises()

		expect(store.updateProcessor).toHaveBeenCalled()
		expect(submittedPayload.id).toBe('proc-test-1')
		expect(submittedPayload.payload.name).toBe('Drop Meta Attributes')
		expect(submittedPayload.payload.config.module).toBe('drop_attributes')
		expect(submittedPayload.payload.config.config.keys).toEqual({ record: ['debug.info'] })
	})

	it('merender ProcessorForm dengan modul attribute_filter pada mode UPDATE', async () => {
		const pinia = createPinia()
		setActivePinia(pinia)
		const store = useProcessorStore()
		store.processorFormMode = FormMode.UPDATE
		store.currentProcessor = {
			processor_id: 'proc-test-2',
			name: 'Filter Prod Records',
			adm_status: 1,
			config: {
				module: 'attribute_filter',
				config: {
					match: {
						key: 'env',
						key_is_regex: false,
						op: '=',
						table: 'record',
						value: { string: 'prod' }
					}
				}
			}
		} as any

		const wrapper = mount(ProcessorForm, {
			global: {
				plugins: [pinia],
				stubs: commonStubs
			}
		})

		await flushPromises()

		const vm = wrapper.vm as any
		expect(vm.form.config.module).toBe('attribute_filter')
		// Check that AttributeFilterControl is rendered
		const filterComp = wrapper.findComponent({ name: 'AttributeFilterControl' })
		expect(filterComp.exists()).toBe(true)
	})

	it('memvalidasi add_attributes dengan default overwrite: false serta mempertahankan overwrite: true saat diaktifkan', () => {
		const draft = AutoFormAdapter.toDraft(AddAttributes, null, processorHints)
		expect(draft.overwrite).toBe(false)

		const defaultResult = AutoFormAdapter.toRequest(
			CreateProcessorRequest,
			{
				name: 'Enrich Logs Processor',
				adm_status: 1,
				input_items: [],
				input_processors: [],
				config: {
					type: 'add_attributes',
					config: {
						kv_map: {
							record: { 'cluster.region': { type: 'str', value: 'ap-southeast-1' } }
						}
					}
				}
			},
			processorHints
		)
		expect(defaultResult.success).toBe(true)
		if (defaultResult.success) {
			expect((defaultResult.data.config as any).config.overwrite).toBe(false)
		}

		const trueResult = AutoFormAdapter.sanitizeAndValidate(
			AddAttributes,
			{
				overwrite: true,
				kv_map: { record: { 'service.name': { type: 'str', value: 'auth-service' } } }
			},
			processorHints
		)
		expect(trueResult.isValid).toBe(true)
		expect(trueResult.data?.overwrite).toBe(true)
	})

	it('merender ProcessorForm dengan modul transform (OPL) pada mode UPDATE dan submit berhasil', async () => {
		const pinia = createPinia()
		setActivePinia(pinia)
		const store = useProcessorStore()
		store.processorFormMode = FormMode.UPDATE
		store.currentProcessor = {
			processor_id: 'proc-test-opl',
			name: 'Transform Metrics OPL',
			adm_status: 1,
			config: {
				module: 'transform',
				config: {
					opl: 'metrics | where name == "http_requests_total" | set attributes["environment"] = "production"'
				}
			}
		} as any

		let submittedPayload: any = null
		store.updateProcessor = vi.fn().mockImplementation((id, payload, clusters) => {
			submittedPayload = { id, payload, clusters }
			return Promise.resolve({ success: true })
		})

		const wrapper = mount(ProcessorForm, {
			global: {
				plugins: [pinia],
				stubs: {
					...commonStubs,
					AppInputCode: {
						props: ['modelValue', 'language', 'label', 'required', 'error'],
						template:
							'<div class="stub-input-code" :data-val="modelValue">{{ modelValue }}</div>'
					}
				}
			}
		})

		await flushPromises()

		const vm = wrapper.vm as any
		expect(vm.form.config.module).toBe('transform')
		expect(vm.form.config.config.opl).toContain('http_requests_total')

		await vm.handleFormSubmit(vm.form)
		await flushPromises()

		expect(store.updateProcessor).toHaveBeenCalled()
		expect(submittedPayload.payload.config.config.opl).toBe(
			'metrics | where name == "http_requests_total" | set attributes["environment"] = "production"'
		)
	})

	it('memblokir submit transform processor jika sintaks OPL tidak valid', async () => {
		const pinia = createPinia()
		setActivePinia(pinia)
		const store = useProcessorStore()

		store.processorFormMode = FormMode.UPDATE
		store.currentProcessor = {
			processor_id: 'proc-test-opl-invalid',
			name: 'Transform Invalid OPL',
			adm_status: 1,
			config: {
				module: 'transform',
				config: {
					opl: 'logs | where'
				}
			}
		} as any

		store.updateProcessor = vi.fn()

		const wrapper = mount(ProcessorForm, {
			global: {
				plugins: [pinia],
				stubs: {
					...commonStubs,
					AppInputCode: {
						props: ['modelValue', 'language', 'label', 'required', 'error'],
						template: '<div class="stub-input-code">{{ modelValue }}</div>'
					}
				}
			}
		})

		await flushPromises()

		const vm = wrapper.vm as any
		await vm.handleFormSubmit(vm.form)
		await flushPromises()

		expect(store.updateProcessor).not.toHaveBeenCalled()
		expect(vm.dynamicFormRef.issues['config.config.opl']).toBeDefined()
		expect(vm.dynamicFormRef.issues['config.config.opl']).toContain(
			"Operator 'where' membutuhkan ekspresi kondisi"
		)
	})
})

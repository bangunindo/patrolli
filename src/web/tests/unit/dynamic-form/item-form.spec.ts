// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import '@@/shared/types/array.type'
import { describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import ItemForm from '@/components/configuration/items/Form.vue'
import { useItemsStore } from '@/stores/items.store'
import { FormMode } from '@@/shared/types/app.type'
import { ItemCategory, ItemType } from '@@/shared/types/item.type'
import DynamicZodForm from '@/components/app/dynamic-zod-form/DynamicZodForm.vue'
import AppInputJson from '@/components/app/input/Json.vue'

vi.mock('vue-router', () => ({
	useRoute: () => ({ query: {}, params: {} })
}))

describe('Item DynamicZodForm Integration', () => {
	const commonStubs = {
		'b-card': {
			template: '<div><slot name="header"/><slot/><slot name="footer"/></div>'
		},
		'b-icon': true,
		'b-heading': { template: '<div><slot/></div>' },
		'b-checkbox': { template: '<div><slot/></div>' },
		'b-flex': { template: '<div><slot/></div>' },
		'b-flex-item': { template: '<div><slot/></div>' },
		'b-button': {
			template: '<button type="button"><slot/></button>'
		},
		UiSwitch: true,
		'ui-switch': true,
		'b-switch': true,
		'b-alert': true,
		AppInputCluster: true,
		AppInputHost: true,
		AppInputSelect: true,
		AppInputEndpoint: true,
		AppInputListener: true,
		AppInputLabel: true,
		AppInputText: true,
		AppInputJson: true
	}

	it('merender ItemForm dengan DynamicZodForm pada mode ADD untuk Host Item', async () => {
		const pinia = createPinia()
		setActivePinia(pinia)
		const store = useItemsStore()
		store.itemsFormMode = FormMode.ADD

		const wrapper = mount(ItemForm, {
			props: {
				isTemplateContext: false
			},
			global: {
				plugins: [pinia],
				stubs: commonStubs
			}
		})

		await flushPromises()

		const dynamicZodForm = wrapper.findComponent(DynamicZodForm)
		expect(dynamicZodForm.exists()).toBe(true)

		const vm = wrapper.vm as any
		expect(vm.form.adm_status).toBe(1)
		expect(vm.form.flags).toBe(0)
		expect(wrapper.text()).toContain('Create Item for Host')
		expect(wrapper.text()).not.toContain('No applicable renderer found')
		expect(
			wrapper.findComponent({ name: 'UiSwitch' }).exists() ||
				wrapper.findComponent({ name: 'b-switch' }).exists() ||
				wrapper.html().includes('switch')
		).toBe(true)
		expect(
			wrapper.findComponent({ name: 'AppInputLabel' }).exists() ||
				wrapper.html().includes('app-input-label')
		).toBe(true)
	})

	it('merender ItemForm dengan DynamicZodForm pada mode ADD untuk Template Item', async () => {
		const pinia = createPinia()
		setActivePinia(pinia)
		const store = useItemsStore()
		store.itemsFormMode = FormMode.ADD

		const wrapper = mount(ItemForm, {
			props: {
				isTemplateContext: true
			},
			global: {
				plugins: [pinia],
				stubs: commonStubs
			}
		})

		await flushPromises()

		const dynamicZodForm = wrapper.findComponent(DynamicZodForm)
		expect(dynamicZodForm.exists()).toBe(true)

		const vm = wrapper.vm as any
		expect(vm.isTemplateContext).toBe(true)
		expect(wrapper.text()).toContain('Create Item for Template')
		expect(wrapper.text()).not.toContain('No applicable renderer found')
	})

	it('merender ItemForm dengan SNMP pada mode UPDATE dan memvalidasi submit payload', async () => {
		const pinia = createPinia()
		setActivePinia(pinia)
		const store = useItemsStore()
		store.itemsFormMode = FormMode.UPDATE
		store.currentItem = {
			item_id: 'item-host-1',
			name: 'CPU Load Item',
			schedule: '30s',
			timeout_ms: 2000,
			adm_status: 1,
			flags: 0,
			host_id: 'host-123',
			endpoint_id: '11111111-2222-3333-4444-555555555555',
			labels: { env: 'prod' },
			config: {
				type: 'snmp',
				config: {
					oid: '1.3.6.1.4.1.2021.10.1.3.1'
				}
			}
		} as any

		let submittedPayload: any = null
		store.updateItemHost = vi.fn().mockImplementation((id, payload, clusters) => {
			submittedPayload = { id, payload, clusters }
			return Promise.resolve({ success: true })
		})

		const wrapper = mount(ItemForm, {
			props: {
				isTemplateContext: false
			},
			global: {
				plugins: [pinia],
				stubs: commonStubs
			}
		})

		await flushPromises()

		const vm = wrapper.vm as any
		expect(vm.form.name).toBe('CPU Load Item')
		expect(vm.form.host_id).toBe('host-123')
		expect(vm.form.schedule).toBe('30s')
		expect(vm.form.timeout_ms).toBe(2000)
		expect(vm.form.config.type).toBe('snmp')
		expect(vm.form.config.config.oid).toBe('1.3.6.1.4.1.2021.10.1.3.1')
		expect(vm.isEndpointRequiredForType).toBe(true)
		expect(vm.isListenerRequiredForType).toBe(false)

		const hostInput = wrapper.findComponent({ name: 'AppInputHost' })
		expect(hostInput.exists()).toBe(true)
		expect(hostInput.props('modelValue')).toBe('host-123')

		// Submit form
		await vm.handleFormSubmit(vm.form)
		await flushPromises()

		expect(store.updateItemHost).toHaveBeenCalled()
		expect(submittedPayload.id).toBe('item-host-1')
		expect(submittedPayload.payload.name).toBe('CPU Load Item')
		expect(submittedPayload.payload.endpoint_id).toBe('11111111-2222-3333-4444-555555555555')
		expect(submittedPayload.payload.config.config.oid).toBe('1.3.6.1.4.1.2021.10.1.3.1')
	})

	it('mendeserialisasi data dummy item yang ada (attrs tuple array, body object, dan ISO datetime) ke format form', async () => {
		const pinia = createPinia()
		setActivePinia(pinia)
		const store = useItemsStore()
		store.itemsFormMode = FormMode.UPDATE
		store.currentItem = {
			item_id: 'dummy-item-1',
			name: 'Existing Dummy Item',
			schedule: '60s',
			timeout_ms: 5000,
			adm_status: 1,
			flags: 0,
			host_id: 'host-111',
			labels: {},
			config: {
				type: 'dummy',
				config: {
					count: 2,
					payload: {
						row: {
							observed_time: '2026-10-07T12:34:56Z',
							severity: 'warn',
							source_ip: '10.0.0.1',
							source_port: 8080,
							body: { String: 'existing warning log' },
							attrs: [[{ String: 'cluster' }, { String: { String: 'sg-prod' } }]]
						}
					}
				}
			}
		} as any

		const wrapper = mount(ItemForm, {
			props: { isTemplateContext: false },
			global: { plugins: [pinia], stubs: commonStubs }
		})

		await flushPromises()
		const vm = wrapper.vm as any

		expect(vm.form.name).toBe('Existing Dummy Item')
		expect(vm.form.config.type).toBe('dummy')
		const row = vm.form.config.config.payload.row
		expect(row.observed_time).toBe('2026-10-07')
		expect(row.body).toEqual({ String: 'existing warning log' })
		expect(row.attrs).toEqual([[{ String: 'cluster' }, { String: { String: 'sg-prod' } }]])
		expect(row.severity).toBe('warn')
		expect(row.source_ip).toBe('10.0.0.1')
		expect(row.source_port).toBe(8080)
	})

	it('mempersistensikan dan menserialisasikan typed attributes (String, Int64, Double, Bool, Null) sesuai Zod LogRowOwned schema', async () => {
		const pinia = createPinia()
		setActivePinia(pinia)
		const store = useItemsStore()
		store.itemsFormMode = FormMode.ADD

		let submittedPayload: any = null
		store.createItemForHost = vi.fn().mockImplementation((hostId, payload, clusters) => {
			submittedPayload = { hostId, payload, clusters }
			return Promise.resolve({ success: true })
		})

		const wrapper = mount(ItemForm, {
			props: { isTemplateContext: false },
			global: { plugins: [pinia], stubs: commonStubs }
		})

		await flushPromises()
		const vm = wrapper.vm as any

		vm.form.name = 'Typed Dummy Attrs Item'
		vm.form.schedule = '15s'
		vm.form.timeout_ms = 2000
		vm.form.host_id = '22222222-2222-2222-2222-222222222222'

		vm.form.config = {
			type: ItemType.DUMMY,
			config: {
				count: 1,
				payload: {
					row: {
						observed_time: '2026-10-07',
						severity: 'debug',
						source_ip: '10.10.10.10',
						source_port: 9090,
						body: 'typed attributes test',
						attrs: [
							[{ String: 'service' }, { String: { String: 'payments' } }],
							[{ String: 'status_code' }, { Int64: 200 }],
							[{ String: 'duration_ms' }, { Double: 12.34 }],
							[{ String: 'is_active' }, { Bool: true }],
							[{ String: 'optional_meta' }, 'Null']
						]
					}
				}
			}
		}
		await flushPromises()

		expect(wrapper.text()).not.toContain('No applicable renderer found')

		await vm.handleFormSubmit(vm.form)
		await flushPromises()

		expect(store.createItemForHost).toHaveBeenCalled()
		const row = submittedPayload.payload.config.config.payload.row
		expect(row.attrs).toEqual([
			[{ String: 'service' }, { String: { String: 'payments' } }],
			[{ String: 'status_code' }, { Int64: 200 }],
			[{ String: 'duration_ms' }, { Double: 12.34 }],
			[{ String: 'is_active' }, { Bool: true }],
			[{ String: 'optional_meta' }, 'Null']
		])
	})

	it('mempersistensikan dan menserialisasikan field body sesuai ZStringOwned schema ({ String: string } atau null)', async () => {
		const pinia = createPinia()
		setActivePinia(pinia)
		const store = useItemsStore()
		store.itemsFormMode = FormMode.ADD

		let submittedPayload: any = null
		store.createItemForHost = vi.fn().mockImplementation((hostId, payload, clusters) => {
			submittedPayload = { hostId, payload, clusters }
			return Promise.resolve({ success: true })
		})

		const wrapper = mount(ItemForm, {
			props: { isTemplateContext: false },
			global: { plugins: [pinia], stubs: commonStubs }
		})

		await flushPromises()
		const vm = wrapper.vm as any

		vm.form.name = 'Dummy Body Test'
		vm.form.schedule = '10s'
		vm.form.timeout_ms = 1000
		vm.form.host_id = '33333333-3333-3333-3333-333333333333'

		// Case A: Body as object { String: 'Sample log body' }
		vm.form.config = {
			type: ItemType.DUMMY,
			config: {
				count: 1,
				payload: {
					row: {
						body: { String: 'Sample log body' },
						attrs: []
					}
				}
			}
		}
		await flushPromises()

		await vm.handleFormSubmit(vm.form)
		await flushPromises()
		expect(submittedPayload.payload.config.config.payload.row.body).toEqual({
			String: 'Sample log body'
		})

		// Case B: Body as empty/whitespace -> serializes to null
		vm.form.config.config.payload.row.body = '   '
		await vm.handleFormSubmit(vm.form)
		await flushPromises()
		expect(submittedPayload.payload.config.config.payload.row.body).toBeNull()

		// Case C: Body as SmolStr object -> preserves { SmolStr: 'From smolstr' }
		vm.form.config.config.payload.row.body = { SmolStr: 'From smolstr' }
		await vm.handleFormSubmit(vm.form)
		await flushPromises()
		expect(submittedPayload.payload.config.config.payload.row.body).toEqual({
			SmolStr: 'From smolstr'
		})
	})

	it('berhasil melakukan validasi Zod dan submit Dummy item saat dipicu via triggerSubmit() pada form baru', async () => {
		const pinia = createPinia()
		setActivePinia(pinia)
		const store = useItemsStore()
		store.itemsFormMode = FormMode.ADD

		let submittedPayload: any = null
		store.createItemForHost = vi.fn().mockImplementation((hostId, payload) => {
			submittedPayload = { hostId, payload }
			return Promise.resolve({ item_id: 'dummy-created-1' })
		})

		const wrapper = mount(ItemForm, {
			props: {
				isTemplateContext: false
			},
			global: {
				plugins: [pinia],
				stubs: commonStubs
			}
		})
		await flushPromises()

		const vm = wrapper.vm as any
		// Isi data mandatory
		vm.form.host_id = 'test-host-id'
		vm.form.name = 'Dummy Through TriggerSubmit'
		vm.form.schedule = '10s'
		vm.form.timeout_ms = 3000
		vm.form.config = {
			type: 'dummy',
			config: {
				count: 1,
				payload: {
					row: {
						observed_time: '2026-10-07T00:00:00Z',
						severity: 'info',
						source_ip: '192.168.1.100',
						source_port: 514,
						body: { String: 'Hello dummy log' },
						attrs: [
							[{ String: 'service' }, { String: { String: 'auth' } }],
							[{ String: 'tier' }, { String: { String: 'backend' } }]
						]
					}
				}
			}
		}
		await flushPromises()

		expect(wrapper.text()).not.toContain('No applicable renderer found')
		const jsonInput = wrapper.findComponent(AppInputJson)
		expect(jsonInput.exists()).toBe(true)

		const dynamicForm = wrapper.findComponent({ name: 'DynamicZodForm' })
		expect(dynamicForm.exists()).toBe(true)

		// Call triggerSubmit() which exposes validation and form submit
		await vm.triggerSubmit()
		await flushPromises()

		expect(store.createItemForHost).toHaveBeenCalledTimes(1)
		expect(submittedPayload.payload.name).toBe('Dummy Through TriggerSubmit')
		expect(submittedPayload.payload.config.type).toBe('dummy')
		const row = submittedPayload.payload.config.config.payload.row
		expect(row.observed_time).toBe('2026-10-07T00:00:00Z')
		expect(row.severity).toBe('info')
		expect(row.source_ip).toBe('192.168.1.100')
		expect(row.source_port).toBe(514)
		expect(row.body).toEqual({ String: 'Hello dummy log' })
		expect(Array.isArray(row.attrs)).toBe(true)
		expect(row.attrs).toHaveLength(2)
		expect(row.attrs).toContainEqual([{ String: 'service' }, { String: { String: 'auth' } }])
		expect(row.attrs).toContainEqual([{ String: 'tier' }, { String: { String: 'backend' } }])
	})

	it('mencegah submit dan menampilkan hostError jika host_id kosong pada mode ADD Host Item', async () => {
		const pinia = createPinia()
		setActivePinia(pinia)
		const store = useItemsStore()
		store.itemsFormMode = FormMode.ADD

		store.createItemForHost = vi.fn()

		const wrapper = mount(ItemForm, {
			props: {
				isTemplateContext: false
			},
			global: {
				plugins: [pinia],
				stubs: commonStubs
			}
		})
		await flushPromises()

		const vm = wrapper.vm as any
		vm.form.host_id = ''
		vm.form.name = 'Test Item Without Host'
		vm.form.schedule = '10s'
		vm.form.timeout_ms = 1000

		await vm.triggerSubmit()
		await flushPromises()

		const hostInput = wrapper.findComponent({ name: 'AppInputHost' })
		expect(hostInput.props('error')).toBe('Host is required')
	})

	it('menyembunyikan endpoint, menampilkan listener, dan memvalidasi listener_id saat module diubah ke listener', async () => {
		const pinia = createPinia()
		setActivePinia(pinia)
		const store = useItemsStore()
		store.itemsFormMode = FormMode.ADD

		let submittedPayload: any = null
		store.createItemForHost = vi.fn().mockImplementation((hostId, payload) => {
			submittedPayload = { hostId, payload }
			return Promise.resolve({ item_id: 'listener-item-1' })
		})

		const wrapper = mount(ItemForm, {
			props: { isTemplateContext: false },
			global: { plugins: [pinia], stubs: commonStubs }
		})
		await flushPromises()

		const vm = wrapper.vm as any
		vm.form.host_id = 'test-host-id'
		vm.form.name = 'Listener Item Test'
		vm.form.schedule = '30s'
		vm.form.timeout_ms = 1000
		vm.form.endpoint_id = 'previous-endpoint-id'

		const dynamicForm = wrapper.findComponent(DynamicZodForm)
		// Update collector module via DynamicZodForm to listener
		dynamicForm.vm.$emit('update:modelValue', {
			...vm.form,
			config: { type: 'listener', config: {} }
		})
		await flushPromises()

		expect(vm.form.config.type).toBe('listener')
		expect(vm.isEndpointRequiredForType).toBe(false)
		expect(vm.isListenerRequiredForType).toBe(true)
		expect(vm.form.endpoint_id).toBeUndefined()

		// Attempt submit without listener_id -> should block and set error
		await vm.triggerSubmit()
		await flushPromises()

		expect(store.createItemForHost).not.toHaveBeenCalled()
		const listenerInput = wrapper.findComponent({ name: 'AppInputListener' })
		expect(listenerInput.props('error')).toBe('Listener is required')

		// Provide listener_id and submit
		vm.form.listener_id = '22222222-2222-2222-2222-222222222222'
		await vm.triggerSubmit()
		await flushPromises()

		expect(store.createItemForHost).toHaveBeenCalledTimes(1)
		expect(submittedPayload.payload.listener_id).toBe('22222222-2222-2222-2222-222222222222')
		expect(submittedPayload.payload.endpoint_id).toBeUndefined()
		expect(submittedPayload.payload.config.type).toBe('listener')
	})

	it('menyembunyikan receiver dan langsung bisa submit saat module diubah ke dummy', async () => {
		const pinia = createPinia()
		setActivePinia(pinia)
		const store = useItemsStore()
		store.itemsFormMode = FormMode.ADD

		let submittedPayload: any = null
		store.createItemForHost = vi.fn().mockImplementation((hostId, payload) => {
			submittedPayload = { hostId, payload }
			return Promise.resolve({ item_id: 'dummy-item-direct' })
		})

		const wrapper = mount(ItemForm, {
			props: { isTemplateContext: false },
			global: { plugins: [pinia], stubs: commonStubs }
		})
		await flushPromises()

		const vm = wrapper.vm as any
		vm.form.host_id = 'test-host-id'
		vm.form.name = 'Direct Dummy Item'
		vm.form.schedule = '15s'
		vm.form.timeout_ms = 1000
		vm.form.endpoint_id = 'some-endpoint-id'

		const dynamicForm = wrapper.findComponent(DynamicZodForm)
		// Update collector module via DynamicZodForm to dummy with default dummy structure
		dynamicForm.vm.$emit('update:modelValue', {
			...vm.form,
			config: {
				type: 'dummy',
				config: {
					count: 1,
					payload: {
						row: {
							observed_time: null,
							severity: 'info',
							source_ip: null,
							source_port: null,
							body: null,
							attrs: []
						}
					}
				}
			}
		})
		await flushPromises()

		// Verify receiver fields are hidden and endpoint_id is cleared
		expect(vm.form.config.type).toBe('dummy')
		expect(vm.isEndpointRequiredForType).toBe(false)
		expect(vm.isListenerRequiredForType).toBe(false)
		expect(vm.form.endpoint_id).toBeUndefined()
		expect(vm.form.listener_id).toBeUndefined()

		// Direct submit without needing any manual dummy row editing
		await vm.triggerSubmit()
		await flushPromises()

		expect(store.createItemForHost).toHaveBeenCalledTimes(1)
		expect(submittedPayload.payload.name).toBe('Direct Dummy Item')
		expect(submittedPayload.payload.config.type).toBe('dummy')
		expect(submittedPayload.payload.config.config.payload.row.severity).toBe('info')
		expect(submittedPayload.payload.endpoint_id).toBeUndefined()
		expect(submittedPayload.payload.listener_id).toBeUndefined()
	})

	it('merender form edit untuk Applied Item Template dengan urutan field yang sama dan menampilkan resolved_schedule serta resolved_timeout_ms sebagai placeholder', async () => {
		const pinia = createPinia()
		setActivePinia(pinia)
		const store = useItemsStore()
		store.itemsFormMode = FormMode.UPDATE
		store.currentItem = {
			item_id: 'applied-item-123',
			host_id: 'host-uuid-1',
			template_item_id: 'tmpl-item-uuid-1',
			name: null,
			resolved_name: 'Inherited Root Item',
			schedule: null,
			resolved_schedule: '30s',
			timeout_ms: null,
			resolved_timeout_ms: 1000,
			flags: 0,
			adm_status: 1,
			endpoint_id: '11111111-1111-4111-a111-111111111111',
			labels: {},
			config: {
				type: 'snmp',
				config: {
					oid: '1.3.6.1.2.1.1.1.0'
				}
			}
		} as any

		store.fetchItemById = vi.fn().mockResolvedValue(store.currentItem)
		store.fetchHosts = vi.fn().mockResolvedValue([])
		store.fetchTemplates = vi.fn().mockResolvedValue([])

		let submittedPayload: any = null
		store.updateItemApliedTemplate = vi.fn().mockImplementation((itemId, payload) => {
			submittedPayload = { itemId, payload }
			return Promise.resolve({ item_id: itemId })
		})

		const wrapper = mount(ItemForm, {
			props: { isTemplateContext: false },
			global: { plugins: [pinia], stubs: commonStubs }
		})
		await flushPromises()

		const vm = wrapper.vm as any

		// Category must be APPLIED_TEMPLATE
		expect(vm.category).toBe(ItemCategory.APPLIED_TEMPLATE)

		// Field orders must match edit items order (name=1, flags=2, schedule=3, timeout_ms=4, endpoint_id=5, listener_id=6, adm_status=7, labels=8, config=9)
		expect(vm.activeHints.name?.order).toBe(1)
		expect(vm.activeHints.flags?.order).toBe(2)
		expect(vm.activeHints.schedule?.order).toBe(3)
		expect(vm.activeHints.timeout_ms?.order).toBe(4)
		expect(vm.activeHints.endpoint_id?.order).toBe(5)
		expect(vm.activeHints.listener_id?.order).toBe(6)
		expect(vm.activeHints.adm_status?.order).toBe(7)
		expect(vm.activeHints.labels?.order).toBe(8)
		expect(vm.activeHints.config?.order).toBe(9)

		// Placeholders must reflect resolved values with "Inherited: "
		expect(vm.activeHints.name?.placeholder).toBe('Inherited: Inherited Root Item')
		expect(vm.activeHints.schedule?.placeholder).toBe('Inherited: 30s')
		expect(vm.activeHints.timeout_ms?.placeholder).toBe('Inherited: 1000')

		// Labels must be retained
		expect(vm.activeHints.schedule?.label).toBe('Schedule (Seconds)')
		expect(vm.activeHints.timeout_ms?.label).toBe('Timeout (ms)')

		// Form values must remain null so they inherit if unchanged
		expect(vm.form.name).toBe('')
		expect(vm.form.schedule).toBeNull()
		expect(vm.form.timeout_ms).toBeNull()

		// Host field must be disabled / read-only for Applied Item Template
		const hostInput = wrapper.findComponent({ name: 'AppInputHost' })
		expect(hostInput.exists()).toBe(true)
		expect(hostInput.props('disabled')).toBe(true)

		// Ensure timeout_ms renders as a number input, not as a union checkbox
		expect(wrapper.find('#union-check-timeout_ms').exists()).toBe(false)
		const timeoutInput = wrapper
			.findAllComponents({ name: 'AppInputText' })
			.find((c) => c.props('label') === 'Timeout (ms)')
		expect(timeoutInput?.exists()).toBe(true)
		expect(timeoutInput?.props('type')).toBe('number')
		expect(timeoutInput?.props('placeholder')).toBe('Inherited: 1000')
		expect(timeoutInput?.props('required')).toBe(false)
		expect(timeoutInput?.props('error')).toBe('')

		const scheduleInput = wrapper
			.findAllComponents({ name: 'AppInputText' })
			.find((c) => c.props('label') === 'Schedule (Seconds)')
		expect(scheduleInput?.exists()).toBe(true)
		expect(scheduleInput?.props('required')).toBe(false)
		expect(scheduleInput?.props('error')).toBe('')

		// Submitting via triggerSubmit() without changes submits null for inherited fields and passes validation
		await vm.triggerSubmit()
		await flushPromises()

		expect(store.updateItemApliedTemplate).toHaveBeenCalledTimes(1)
		expect(submittedPayload.itemId).toBe('applied-item-123')
		expect(submittedPayload.payload.name).toBeNull()
		expect(submittedPayload.payload.schedule).toBeNull()
		expect(submittedPayload.payload.timeout_ms).toBeNull()
	})

	it('mengirim nilai baru saat user mengisi override pada schedule dan timeout_ms di Applied Item Template', async () => {
		const pinia = createPinia()
		setActivePinia(pinia)
		const store = useItemsStore()
		store.itemsFormMode = FormMode.UPDATE
		store.currentItem = {
			item_id: 'applied-item-456',
			host_id: 'host-uuid-1',
			template_item_id: 'tmpl-item-uuid-1',
			name: null,
			resolved_name: 'Inherited Root Item',
			schedule: null,
			resolved_schedule: '30s',
			timeout_ms: null,
			resolved_timeout_ms: 1000,
			flags: 0,
			adm_status: 1,
			labels: {},
			config: {
				type: 'snmp',
				config: {
					oid: '1.3.6.1.2.1.1.1.0'
				}
			}
		} as any

		store.fetchItemById = vi.fn().mockResolvedValue(store.currentItem)
		store.fetchHosts = vi.fn().mockResolvedValue([])
		store.fetchTemplates = vi.fn().mockResolvedValue([])

		let submittedPayload: any = null
		store.updateItemApliedTemplate = vi.fn().mockImplementation((itemId, payload) => {
			submittedPayload = { itemId, payload }
			return Promise.resolve({ item_id: itemId })
		})

		const wrapper = mount(ItemForm, {
			props: { isTemplateContext: false },
			global: { plugins: [pinia], stubs: commonStubs }
		})
		await flushPromises()

		const vm = wrapper.vm as any

		// User overrides fields
		vm.form.schedule = '60s'
		vm.form.timeout_ms = 5000
		vm.form.name = 'Custom Name Override'

		await vm.handleFormSubmit(vm.form)
		await flushPromises()

		expect(store.updateItemApliedTemplate).toHaveBeenCalledTimes(1)
		expect(submittedPayload.itemId).toBe('applied-item-456')
		expect(submittedPayload.payload.name).toBe('Custom Name Override')
		expect(submittedPayload.payload.schedule).toBe('60s')
		expect(submittedPayload.payload.timeout_ms).toBe(5000)
	})
})

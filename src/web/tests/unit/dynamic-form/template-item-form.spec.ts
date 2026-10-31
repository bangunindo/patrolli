// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import '@@/shared/types/array.type'
import { describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import ItemFormTemplate from '@/components/configuration/items/FormTemplate.vue'
import ItemForm from '@/components/configuration/items/Form.vue'
import { useItemsStore } from '@/stores/items.store'
import { FormMode } from '@@/shared/types/app.type'
import { ItemType } from '@@/shared/types/item.type'
import DynamicZodForm from '@/components/app/dynamic-zod-form/DynamicZodForm.vue'

vi.mock('vue-router', () => ({
	useRoute: () => ({ query: {}, params: {} })
}))

describe('Item Template Form DynamicZodForm Integration', () => {
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
		'b-switch': true,
		'b-alert': true,
		AppInputCluster: true,
		AppInputHost: true,
		AppInputSelect: true,
		AppInputEndpoint: true,
		AppInputListener: true,
		AppInputLabel: true,
		AppInputText: true
	}

	it('merender ItemFormTemplate dengan DynamicZodForm pada mode ADD', async () => {
		const pinia = createPinia()
		setActivePinia(pinia)
		const store = useItemsStore()
		store.itemsFormMode = FormMode.ADD

		const wrapper = mount(ItemFormTemplate, {
			global: {
				plugins: [pinia],
				stubs: commonStubs
			}
		})

		await flushPromises()

		const dynamicZodForm = wrapper.findComponent(DynamicZodForm)
		expect(dynamicZodForm.exists()).toBe(true)

		const itemForm = wrapper.findComponent(ItemForm)
		expect(itemForm.exists()).toBe(true)
		const vm = itemForm.vm as any

		expect(vm.isTemplateContext).toBe(true)
		expect(wrapper.text()).toContain('Create Item for Template')
		// Ensure AppInputHost is not displayed in template context
		expect(wrapper.findComponent({ name: 'AppInputHost' }).exists()).toBe(false)
	})

	it('berhasil memvalidasi dan memanggil createItemForTemplate saat submit template item', async () => {
		const pinia = createPinia()
		setActivePinia(pinia)
		const store = useItemsStore()
		store.itemsFormMode = FormMode.ADD

		let submittedPayload: any = null
		store.createItemForTemplate = vi
			.fn()
			.mockImplementation((templateId, payload, clusters) => {
				submittedPayload = { templateId, payload, clusters }
				return Promise.resolve({ success: true })
			})

		const wrapper = mount(ItemFormTemplate, {
			global: {
				plugins: [pinia],
				stubs: commonStubs
			}
		})

		await flushPromises()
		const itemForm = wrapper.findComponent(ItemForm)
		const vm = itemForm.vm as any

		vm.form.name = 'CPU Usage Template Item'
		vm.form.schedule = '60s'
		vm.form.timeout_ms = 5000
		vm.form.template_id = 'tmpl-parent-1234'
		vm.form.config = {
			type: ItemType.SNMP,
			config: {
				oid: '1.3.6.1.2.1.1.3.0'
			}
		}
		await flushPromises()

		await vm.handleFormSubmit(vm.form)
		await flushPromises()

		expect(store.createItemForTemplate).toHaveBeenCalled()
		expect(submittedPayload.templateId).toBe('tmpl-parent-1234')
		expect(submittedPayload.payload.name).toBe('CPU Usage Template Item')
		expect(submittedPayload.payload.schedule).toBe('60s')
		expect(submittedPayload.payload.timeout_ms).toBe(5000)
		expect(submittedPayload.payload.config.type).toBe(ItemType.SNMP)
	})

	it('merender ItemFormTemplate pada mode UPDATE untuk template item', async () => {
		const pinia = createPinia()
		setActivePinia(pinia)
		const store = useItemsStore()
		store.itemsFormMode = FormMode.UPDATE
		store.currentItem = {
			item_id: 'item-tmpl-uuid',
			template_id: 'tmpl-parent-1234',
			name: 'Memory Usage Item',
			schedule: '30s',
			timeout_ms: 1000,
			adm_status: 1,
			flags: 0,
			labels: {},
			type: ItemType.SNMP,
			config: {
				type: ItemType.SNMP,
				config: { oid: '1.3.6.1.2.1.25.2.3.1.5' }
			}
		} as any

		const wrapper = mount(ItemFormTemplate, {
			global: {
				plugins: [pinia],
				stubs: commonStubs
			}
		})

		await flushPromises()
		const itemForm = wrapper.findComponent(ItemForm)
		const vm = itemForm.vm as any

		expect(vm.form.name).toBe('Memory Usage Item')
		expect(vm.form.schedule).toBe('30s')
		expect(wrapper.text()).toContain('Edit Item')
	})
})

// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import '@@/shared/types/array.type'
import { describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import TemplateModalForm from '@/components/configuration/templates/ModalForm.vue'
import { useTemplatesStore } from '@/stores/templates.store'
import { FormMode } from '@@/shared/types/app.type'
import DynamicZodForm from '@/components/app/dynamic-zod-form/DynamicZodForm.vue'

describe('Template ModalForm DynamicZodForm Integration', () => {
	const commonStubs = {
		'b-modal': {
			template: '<div><slot/></div>'
		},
		'b-modal-header': {
			template: '<div><slot/></div>'
		},
		'b-modal-body': {
			template: '<div><slot/></div>'
		},
		'b-modal-footer': {
			template: '<div><slot/></div>'
		},
		'b-icon': true,
		'b-heading': { template: '<div><slot/></div>' },
		'b-flex': { template: '<div><slot/></div>' },
		'b-flex-item': { template: '<div><slot/></div>' },
		'b-button': {
			template: '<button type="button"><slot/></button>'
		},
		'b-alert': true,
		AppInputCluster: true,
		AppInputSelect: true,
		AppInputText: true,
		AppInputLabel: true,
		ModalUnlinkTemplate: true
	}

	it('merender TemplateModalForm dengan DynamicZodForm pada mode ADD', async () => {
		const pinia = createPinia()
		setActivePinia(pinia)
		const store = useTemplatesStore()
		store.templateFormMode = FormMode.ADD
		store.templateModalShow = true

		const wrapper = mount(TemplateModalForm, {
			global: {
				plugins: [pinia],
				stubs: commonStubs
			}
		})

		await flushPromises()

		const dynamicZodForm = wrapper.findComponent(DynamicZodForm)
		expect(dynamicZodForm.exists()).toBe(true)

		const vm = wrapper.vm as any
		expect(vm.form.name).toBe('')
		expect(wrapper.text()).toContain('Create Template')
		expect(wrapper.text()).not.toContain('No applicable renderer found')
	})

	it('berhasil memvalidasi dan memanggil createTemplate saat submit form template valid', async () => {
		const pinia = createPinia()
		setActivePinia(pinia)
		const store = useTemplatesStore()
		store.templateFormMode = FormMode.ADD
		store.templateModalShow = true

		let submittedPayload: any = null
		store.createTemplate = vi.fn().mockImplementation((payload, clusters) => {
			submittedPayload = { payload, clusters }
			return Promise.resolve({ success: true, data: { template_id: 'new-tmpl-uuid' } })
		})
		store.fetchTemplates = vi.fn().mockResolvedValue({})

		const wrapper = mount(TemplateModalForm, {
			global: {
				plugins: [pinia],
				stubs: commonStubs
			}
		})

		await flushPromises()
		const vm = wrapper.vm as any

		vm.form.name = 'Linux Server Base'
		vm.form.labels = JSON.stringify({ env: 'production', tier: 'frontend' })
		await flushPromises()

		await vm.handleFormSubmit(vm.form)
		await flushPromises()

		expect(store.createTemplate).toHaveBeenCalled()
		expect(submittedPayload.payload.name).toBe('Linux Server Base')
		expect(submittedPayload.payload.labels).toEqual({ env: 'production', tier: 'frontend' })
	})

	it('merender TemplateModalForm pada mode UPDATE dan menginisialisasi initial data', async () => {
		const pinia = createPinia()
		setActivePinia(pinia)
		const store = useTemplatesStore()
		store.templateFormMode = FormMode.UPDATE
		store.templateModalShow = true
		store.currentTemplate = {
			template_id: 'tmpl-edit-uuid',
			name: 'Existing Template',
			labels: { role: 'worker' },
			links_to_templates: [{ template_id: 'link-1', name: 'Base Template' }]
		} as any

		const wrapper = mount(TemplateModalForm, {
			global: {
				plugins: [pinia],
				stubs: commonStubs
			}
		})

		await flushPromises()
		const vm = wrapper.vm as any

		expect(vm.form.name).toBe('Existing Template')
		expect(vm.formTemplate.template_ids).toEqual(['link-1'])
		expect(wrapper.text()).toContain('Edit Template')
	})
})

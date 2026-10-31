// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import EndpointForm from '@/components/configuration/endpoint/Form.vue'
import AppInputSelect from '@/components/app/input/AppInputSelect.vue'
import UiCheckbox from '@/components/ui/checkbox/Checkbox.vue'

describe('Endpoint Form Union Checkbox & Single Option Defaults', () => {
	setActivePinia(createPinia())

	it('renders b-checkbox for authentication and privacy in SNMP v3', async () => {
		const wrapper = mount(EndpointForm, {
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
					AppInputHost: true
				}
			}
		})

		await flushPromises()

		// 1. Switch protocol to 'snmp_v3'
		const selects = wrapper.findAllComponents(AppInputSelect)
		const protocolSelect = selects.find((s) => s.props('label') === 'Protocol')
		expect(protocolSelect).toBeDefined()

		await protocolSelect!.vm.$emit('update:modelValue', 'snmp_v3')
		await flushPromises()

		// 2. Expect security_model.type select exists and is pre-filled with 'usm'
		const updatedSelects = wrapper.findAllComponents(AppInputSelect)
		const secTypeSelect = updatedSelects.find(
			(s) => s.props('label') === 'Type' || s.props('label') === 'Select Type'
		)
		expect(secTypeSelect).toBeDefined()
		expect(secTypeSelect!.props('modelValue')).toBe('usm')

		// 3. Expect checkboxes for Use Authentication and Use Privacy
		const checkboxes = wrapper.findAllComponents(UiCheckbox)
		const authCheck = checkboxes.find(
			(c) =>
				c.props('id') === 'union-check-config.config.security_model.config.authentication'
		)
		const privCheck = checkboxes.find(
			(c) => c.props('id') === 'union-check-config.config.security_model.config.privacy'
		)

		expect(authCheck).toBeDefined()
		expect(privCheck).toBeDefined()
		expect(authCheck!.props('checked')).toBe(false)
		expect(privCheck!.props('checked')).toBe(false)
		expect(wrapper.text()).toContain('Use Authentication')
		expect(wrapper.text()).toContain('Use Privacy')

		// 4. Toggle Use Authentication checkbox to true
		await authCheck!.vm.$emit('update:checked', true)
		await flushPromises()

		expect(authCheck!.props('checked')).toBe(true)

		// 5. Check if password and protocol for authentication are now rendered
		const passInputs = wrapper.findAllComponents({ name: 'Password' })
		const authPassInput = passInputs.find((p) => p.props('label') === 'Passphrase')
		expect(authPassInput).toBeDefined()

		// 6. Toggle Use Authentication back to false -> hides Passphrase
		await authCheck!.vm.$emit('update:checked', false)
		await flushPromises()

		expect(authCheck!.props('checked')).toBe(false)
	})

	it('keeps AppInputSelect for multi-variant union in HTTP auth', async () => {
		const wrapper = mount(EndpointForm, {
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
					'b-checkbox': true,
					AppInputCluster: true,
					AppInputHost: true
				}
			}
		})

		await flushPromises()

		// Switch protocol to 'http'
		const protocolSelect = wrapper
			.findAllComponents(AppInputSelect)
			.find((s) => s.props('label') === 'Protocol')
		await protocolSelect!.vm.$emit('update:modelValue', 'http')
		await flushPromises()

		// Authentication Type must be a select dropdown (not a checkbox)
		const authSelect = wrapper
			.findAllComponents(AppInputSelect)
			.find((s) => s.props('label') === 'Authentication Type')
		expect(authSelect).toBeDefined()
		expect(authSelect!.props('modelValue')).toBe('none')
	})
})

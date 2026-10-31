// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import { CreateEndpointRequest } from '@@/shared/generated/patrolliAPI.schemas.type'
import { endpointHints } from '@/app/services/forms/ui-hints/endpoint.hints'
import { convertZodToJsonSchema } from '@/app/services/forms/json-schema-converter'
import { hintsToUiSchema } from '@/app/services/forms/hints-to-uischema'
import DynamicZodForm from '@/components/app/dynamic-zod-form/DynamicZodForm.vue'

describe('Dynamic Zod Form (JSON Forms + Zod + BTJ UI)', () => {
	it('mengonversi Zod Schema ke JSON Schema Draft-07 dengan union branches yang valid', () => {
		const jsonSchema = convertZodToJsonSchema(CreateEndpointRequest) as any

		expect(jsonSchema.type).toBe('object')
		expect(jsonSchema.properties).toBeDefined()
		expect(jsonSchema.properties.name).toBeDefined()
		expect(jsonSchema.properties.host_id).toBeDefined()
		expect(jsonSchema.properties.adm_status).toBeDefined()
		expect(jsonSchema.properties.config).toBeDefined()

		// Verify anyOf union for EndpointConfig
		const configSchema = jsonSchema.properties.config
		expect(configSchema.anyOf || configSchema.oneOf).toBeDefined()
		const branches = configSchema.anyOf || configSchema.oneOf
		expect(branches.length).toBe(4)
	})

	it('mengonversi UI Hints menjadi UISchema element secara terurut', () => {
		const jsonSchema = convertZodToJsonSchema(CreateEndpointRequest)
		const uischema = hintsToUiSchema(jsonSchema, endpointHints) as any

		expect(uischema.type).toBe('VerticalLayout')
		expect(Array.isArray(uischema.elements)).toBe(true)

		// First element based on order in endpointHints is name (order: 1)
		const firstElement = uischema.elements[0]
		expect(firstElement.type).toBe('Control')
		expect(firstElement.scope).toBe('#/properties/name')
		expect(firstElement.options?.placeholder).toBe('Name of endpoint')
	})

	it('berhasil memvalidasi dan memicu emit error saat submit data tidak valid', async () => {
		const wrapper = mount(DynamicZodForm, {
			props: {
				schema: CreateEndpointRequest,
				hints: endpointHints,
				modelValue: {
					name: '', // string
					host_id: 'invalid-uuid', // invalid UUID
					adm_status: 1,
					config: {
						type: 'snmp_v1',
						config: {
							address: '',
							community: ''
						}
					}
				}
			},
			global: {
				stubs: {
					'b-alert': true,
					'b-card': true,
					'b-switch': true
				}
			}
		})

		// Trigger submit via method
		wrapper.vm.validateAndSubmit()
		await wrapper.vm.$nextTick()

		// Ensure error emit is called
		expect(wrapper.emitted('error')).toBeDefined()
		const errorPayload = wrapper.emitted('error')![0][0] as Record<string, string>
		expect(Object.keys(errorPayload).length).toBeGreaterThan(0)
		expect(wrapper.emitted('submit')).toBeUndefined()
	})

	it('memunculkan kembali banner alert validasi saat validateAndSubmit dipanggil ulang setelah alert ditutup', async () => {
		const wrapper = mount(DynamicZodForm, {
			props: {
				schema: CreateEndpointRequest,
				hints: endpointHints,
				modelValue: {
					name: '',
					host_id: 'invalid-uuid'
				}
			},
			global: {
				stubs: {
					'b-alert': {
						props: ['title', 'type', 'closeable', 'closeByEvent'],
						emits: ['close'],
						template:
							'<div class="form-alert-mock"><span class="title">{{ title }}</span><button type="button" class="close-banner-btn" @click="$emit(\'close\')">x</button></div>'
					},
					'b-card': true,
					'b-switch': true
				}
			}
		})

		// 1. First submit -> error banner appears
		wrapper.vm.validateAndSubmit()
		await new Promise((r) => setTimeout(r, 50))
		expect(wrapper.find('[data-form-error-banner]').exists()).toBe(true)
		expect(Object.keys(wrapper.vm.issues).length).toBeGreaterThan(0)

		// 2. Close error banner
		await wrapper.find('[data-form-error-banner] button').trigger('click')
		await new Promise((r) => setTimeout(r, 50))
		expect(Object.keys(wrapper.vm.issues).length).toBe(0)
		expect(wrapper.find('[data-form-error-banner]').exists()).toBe(false)

		// 3. Resubmit without changing invalid data -> error banner reappears
		wrapper.vm.validateAndSubmit()
		await new Promise((r) => setTimeout(r, 50))
		expect(wrapper.find('[data-form-error-banner]').exists()).toBe(true)
		expect(Object.keys(wrapper.vm.issues).length).toBeGreaterThan(0)
	})

	it('berhasil memicu emit submit dengan data yang lolos validasi Zod', async () => {
		const validData = {
			name: 'My Endpoint',
			host_id: '123e4567-e89b-12d3-a456-426614174000',
			adm_status: 1,
			config: {
				type: 'snmp_v1',
				config: {
					address: '192.168.1.1:161',
					community: 'public'
				}
			}
		}

		const wrapper = mount(DynamicZodForm, {
			props: {
				schema: CreateEndpointRequest,
				hints: endpointHints,
				modelValue: validData
			},
			global: {
				stubs: {
					'b-alert': true,
					'b-card': true,
					'b-switch': true
				}
			}
		})

		await wrapper.find('form').trigger('submit')

		expect(wrapper.emitted('submit')).toBeDefined()
		const submittedData = wrapper.emitted('submit')![0][0] as any
		expect(submittedData.name).toBe('My Endpoint')
		expect(submittedData.host_id).toBe('123e4567-e89b-12d3-a456-426614174000')
	})
})

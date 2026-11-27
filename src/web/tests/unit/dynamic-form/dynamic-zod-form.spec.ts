// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from 'vitest'
import { h, ref } from 'vue'
import { mount } from '@vue/test-utils'
import { z } from 'zod'
import { CreateEndpointRequest } from '@@/shared/generated/patrolliAPI.schemas.type'
import { endpointHints } from '@/app/services/forms/ui-hints/endpoint.hints'
import { convertZodToJsonSchema } from '@/app/services/forms/json-schema-converter'
import { introspectZodSchema } from '@/app/services/forms/zod-introspector'
import { resolveHintForPath } from '@/app/services/forms/path-helper'
import { listenerHints } from '@/app/services/forms/ui-hints/listener.hints'
import { sinkHints } from '@/app/services/forms/ui-hints/sink.hints'
import DynamicZodForm from '@/components/app/dynamic-zod-form/DynamicZodForm.vue'

describe('Dynamic Zod Form (TanStack Form + Zod + BTJ UI)', () => {
	it('should convert Zod Schema to JSON Schema Draft-07 with valid union branches', () => {
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

	it('should introspect Zod schema with UI Hints into ordered FieldDescriptors', () => {
		const descriptors = introspectZodSchema(CreateEndpointRequest, [], endpointHints)

		expect(Array.isArray(descriptors)).toBe(true)
		expect(descriptors.length).toBeGreaterThan(0)

		// First element based on order in endpointHints is name (order: 1)
		const firstElement = descriptors[0]
		expect(firstElement.name).toBe('name')
		expect(firstElement.pathKey).toBe('name')
		expect(firstElement.placeholder).toBe('Name of endpoint')
	})

	it('should validate and emit error event when submitting invalid data', async () => {
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

	it('should reappear validation alert banner when validateAndSubmit is recalled after alert is closed', async () => {
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

	it('should emit submit event with data passing Zod validation', async () => {
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

	it('should emit update:modelValue only once per field change', async () => {
		const simpleSchema = z.object({
			title: z.string()
		})

		const wrapper = mount(DynamicZodForm, {
			props: {
				schema: simpleSchema,
				modelValue: { title: 'initial' }
			}
		})

		const input = wrapper.find('input')
		await input.setValue('updated')
		await wrapper.vm.$nextTick()

		const emitted = wrapper.emitted('update:modelValue') || []
		expect(emitted.length).toBe(1)
		expect(emitted[0][0]).toEqual({ title: 'updated' })
	})

	it('should not loop or emit update:modelValue when SelectControl has empty string defaultValue and value is already empty', async () => {
		const selectSchema = z.object({
			mode: z.enum(['', 'fast', 'safe'])
		})

		const wrapper = mount(DynamicZodForm, {
			props: {
				schema: selectSchema,
				hints: {
					mode: {
						widget: 'select',
						defaultValue: '',
						options: [
							{ label: 'Default', value: '' },
							{ label: 'Fast', value: 'fast' }
						]
					}
				},
				modelValue: { mode: '' }
			}
		})

		await wrapper.vm.$nextTick()
		expect(wrapper.emitted('update:modelValue')).toBeUndefined()
	})

	it('should support custom field slot on SwitchControl', () => {
		const boolSchema = z.object({
			enabled: z.boolean()
		})

		const wrapper = mount(DynamicZodForm, {
			props: {
				schema: boolSchema,
				modelValue: { enabled: true }
			},
			slots: {
				'field:enabled': (slotProps: any) =>
					h('div', { 'data-testid': 'custom-bool-slot' }, String(slotProps.value))
			}
		})

		const customSlot = wrapper.find('[data-testid="custom-bool-slot"]')
		expect(customSlot.exists()).toBe(true)
		expect(customSlot.text()).toBe('true')
	})

	it('should reset model and clear issues when initialData changes to null', async () => {
		const simpleSchema = z.object({
			title: z.string().min(1, 'Title is required')
		})

		const wrapper = mount(DynamicZodForm, {
			props: {
				schema: simpleSchema,
				initialData: { title: 'existing' }
			}
		})

		expect(wrapper.vm.model).toEqual({ title: 'existing' })

		await wrapper.setProps({ initialData: { title: '' } })
		wrapper.vm.validateAndSubmit()
		await wrapper.vm.$nextTick()
		expect(Object.keys(wrapper.vm.issues).length).toBeGreaterThan(0)

		await wrapper.setProps({ initialData: null })
		await wrapper.vm.$nextTick()
		expect(wrapper.vm.model).toEqual({})
		expect(wrapper.vm.issues).toEqual({})
	})

	it('should update primitive array items using dot path notation', async () => {
		const arraySchema = z.object({
			tags: z.array(z.string())
		})

		const wrapper = mount(DynamicZodForm, {
			props: {
				schema: arraySchema,
				modelValue: { tags: ['alpha'] }
			},
			global: {
				stubs: {
					UiIcon: true,
					UiButton: {
						template:
							'<button type="button" @click="$emit(\'click\')"><slot /></button>'
					}
				}
			}
		})

		const input = wrapper.find('input')
		await input.setValue('beta')
		await wrapper.vm.$nextTick()

		const emitted = wrapper.emitted('update:modelValue') || []
		expect(emitted.length).toBeGreaterThan(0)
		expect(emitted[emitted.length - 1][0]).toEqual({ tags: ['beta'] })
	})

	it('should preserve existing data for single-branch external-tagged union in UnionControl', async () => {
		const singleExternalUnionSchema = z.object({
			auth: z.union([
				z.object({
					basic: z.object({
						username: z.string()
					})
				})
			])
		})

		const wrapper = mount(DynamicZodForm, {
			props: {
				schema: singleExternalUnionSchema,
				initialData: {
					auth: {
						basic: {
							username: 'alice'
						}
					}
				}
			}
		})

		await wrapper.vm.$nextTick()
		expect(wrapper.vm.model).toEqual({
			auth: {
				basic: {
					username: 'alice'
				}
			}
		})
	})

	it('should not render empty GroupLayout when all fields in group are hidden by visibleWhen', async () => {
		const groupedSchema = z.object({
			mode: z.string(),
			secret: z.string().optional()
		})

		const wrapper = mount(DynamicZodForm, {
			props: {
				schema: groupedSchema,
				initialData: { mode: 'off', secret: '' },
				hints: {
					mode: { order: 1 },
					secret: {
						order: 2,
						group: 'Advanced Secret',
						visibleWhen: (data: any) => data?.mode === 'on'
					}
				}
			}
		})

		await wrapper.vm.$nextTick()
		expect(wrapper.text()).not.toContain('Advanced Secret')

		await wrapper.setProps({ initialData: { mode: 'on', secret: 'xyz' } })
		await wrapper.vm.$nextTick()
		expect(wrapper.text()).toContain('Advanced Secret')
	})

	it('should safely clone reactive defaultValue when adding object array items', async () => {
		const { reactive } = await import('vue')
		const objectArraySchema = z.object({
			items: z.array(
				z.object({
					meta: z.object({ tag: z.string() }).default(reactive({ tag: 'default-tag' }))
				})
			)
		})

		const wrapper = mount(DynamicZodForm, {
			props: {
				schema: objectArraySchema,
				modelValue: { items: [] }
			},
			global: {
				stubs: {
					UiIcon: true,
					UiButton: {
						template:
							'<button type="button" @click="$emit(\'click\')"><slot /></button>'
					}
				}
			}
		})

		const addBtn = wrapper.findAll('button').find((b) => b.text().includes('Add'))
		expect(addBtn).toBeDefined()
		await addBtn!.trigger('click')
		await wrapper.vm.$nextTick()

		const emitted = wrapper.emitted('update:modelValue') || []
		expect(emitted.length).toBeGreaterThan(0)
		expect(emitted[emitted.length - 1][0]).toEqual({
			items: [{ meta: { tag: 'default-tag' } }]
		})
	})

	it('should pass disabled state to LabelControl and AppInputLabel', async () => {
		const labelSchema = z.object({
			labels: z.string().optional()
		})

		const wrapper = mount(DynamicZodForm, {
			props: {
				schema: labelSchema,
				hints: {
					labels: { widget: 'label' }
				},
				modelValue: { labels: '' },
				disabled: false
			},
			global: {
				stubs: {
					UiIcon: true,
					UiInput: {
						props: ['modelValue', 'disabled'],
						template: '<input :value="modelValue" :disabled="disabled" />'
					},
					UiButton: {
						props: ['disabled'],
						template:
							'<button type="button" :disabled="disabled" @click="$emit(\'click\')"><slot /></button>'
					}
				}
			}
		})

		const addLabelBtn = wrapper.findAll('button').find((b) => b.text().includes('Add Label'))
		expect(addLabelBtn).toBeDefined()
		await addLabelBtn!.trigger('click')
		await wrapper.vm.$nextTick()

		expect(wrapper.findAll('input').length).toBe(2)

		await wrapper.setProps({ disabled: true })
		await wrapper.vm.$nextTick()

		const inputs = wrapper.findAll('input')
		expect(inputs.length).toBe(2)
		expect(inputs[0].attributes('disabled')).toBeDefined()
		expect(inputs[1].attributes('disabled')).toBeDefined()

		const addLabelBtnAfter = wrapper
			.findAll('button')
			.find((b) => b.text().includes('Add Label'))
		expect(addLabelBtnAfter).toBeUndefined()
	})

	it('should preserve object array row DOM identity when editing child fields', async () => {
		const objectArraySchema = z.object({
			targets: z.array(
				z.object({
					host: z.string()
				})
			)
		})

		const wrapper = mount(DynamicZodForm, {
			props: {
				schema: objectArraySchema,
				initialData: { targets: [{ host: 'a' }] }
			},
			global: {
				stubs: {
					UiIcon: true,
					UiButton: {
						template:
							'<button type="button" @click="$emit(\'click\')"><slot /></button>'
					},
					AppInputText: {
						props: ['modelValue'],
						emits: ['update:modelValue'],
						template:
							'<input class="arr-child-input" :value="modelValue" @input="$emit(\'update:modelValue\', $event.target.value)" />'
					}
				}
			}
		})

		await wrapper.vm.$nextTick()
		const initialInputEl = wrapper.find('input.arr-child-input').element

		await wrapper.find('input.arr-child-input').setValue('ab')
		await wrapper.vm.$nextTick()

		const afterInputEl = wrapper.find('input.arr-child-input').element
		expect(afterInputEl).toBe(initialInputEl)
	})

	it('should remount shared child field when switching union branches', async () => {
		const unionSchema = z.object({
			source: z.discriminatedUnion('type', [
				z.object({
					type: z.literal('tcp'),
					config: z.object({ host: z.string() })
				}),
				z.object({
					type: z.literal('udp'),
					config: z.object({ port: z.number() })
				})
			])
		})

		const wrapper = mount(DynamicZodForm, {
			props: {
				schema: unionSchema,
				initialData: { source: { type: 'tcp', config: { host: '127.0.0.1' } } }
			},
			global: {
				stubs: {
					UiIcon: true,
					AppInputSelect: {
						props: ['modelValue', 'options'],
						emits: ['update:modelValue'],
						template:
							'<select class="union-select" :value="modelValue" @change="$emit(\'update:modelValue\', $event.target.value)"><option v-for="o in options" :key="o.value" :value="o.value">{{ o.label }}</option></select>'
					},
					AppInputText: {
						props: ['modelValue', 'label'],
						template: '<div class="field-stub">{{ label }}:{{ modelValue }}</div>'
					}
				}
			}
		})

		await wrapper.vm.$nextTick()
		expect(wrapper.text()).toContain('Host:127.0.0.1')

		await wrapper.find('select.union-select').setValue('udp')
		await wrapper.vm.$nextTick()

		expect(wrapper.text()).not.toContain('Host:')
		expect(wrapper.text()).toContain('Port:')
	})

	it('should preserve null in SelectControl when emptyPolicy is null even with defaultValue', async () => {
		const selectSchema = z.object({
			severity: z.enum(['info', 'warn']).nullable().optional()
		})

		const wrapper = mount(DynamicZodForm, {
			props: {
				schema: selectSchema,
				hints: {
					severity: {
						defaultValue: 'info',
						emptyPolicy: 'null'
					}
				},
				initialData: { severity: null }
			},
			global: {
				stubs: {
					UiIcon: true,
					AppInputSelect: {
						props: ['modelValue'],
						emits: ['update:modelValue'],
						template:
							'<select class="sev-select" :value="modelValue ?? \'\'" @change="$emit(\'update:modelValue\', $event.target.value)"><option value="">None</option><option value="info">info</option><option value="warn">warn</option></select>'
					}
				}
			}
		})

		await wrapper.vm.$nextTick()
		expect((wrapper.find('select.sev-select').element as HTMLSelectElement).value).toBe('')
		expect((wrapper.vm as any).model.severity).toBeNull()
	})

	it('should render custom slot for PasswordControl when #field slot is provided', async () => {
		const secretSchema = z.object({
			password: z.union([z.string(), z.object({ set: z.string() })])
		})

		const wrapper = mount(DynamicZodForm, {
			props: {
				schema: secretSchema,
				initialData: { password: 'secret' }
			},
			slots: {
				'field:password': '<div class="custom-password-slot">Custom Password</div>'
			},
			global: {
				stubs: {
					UiIcon: true
				}
			}
		})

		await wrapper.vm.$nextTick()
		expect(wrapper.find('.custom-password-slot').exists()).toBe(true)
	})

	it('should flatten nested z.union validation errors to child field paths on submit', async () => {
		const unionSchema = z.object({
			config: z.union([
				z.object({
					type: z.literal('tcp'),
					address: z.string().min(1, 'Address is required')
				}),
				z.object({
					type: z.literal('udp'),
					port: z.number().min(1)
				})
			])
		})

		const wrapper = mount(DynamicZodForm, {
			props: {
				schema: unionSchema,
				initialData: {
					config: {
						type: 'tcp',
						address: ''
					}
				}
			},
			global: {
				stubs: {
					UiIcon: true,
					AppInputSelect: true,
					AppInputText: {
						props: ['modelValue', 'error'],
						template: '<div class="stub-input" :data-error="error" />'
					}
				}
			}
		})

		await (wrapper.vm as any).validateAndSubmit()
		await wrapper.vm.$nextTick()

		const issues = (wrapper.vm as any).issues
		expect(issues['config.address']).toBeTruthy()
		expect(wrapper.find('.stub-input').attributes('data-error')).toBeTruthy()
	})

	it('should render JsonControl with AppInputJson for widget "json" fields', async () => {
		const jsonSchema = z.object({
			metadata: z.unknown()
		})

		const wrapper = mount(DynamicZodForm, {
			props: {
				schema: jsonSchema,
				initialData: { metadata: { foo: 'bar' } }
			},
			global: {
				stubs: {
					UiIcon: true,
					ClientOnly: {
						template: '<slot />'
					},
					AppInputJson: {
						props: ['modelValue', 'label'],
						template: '<div class="stub-json-editor" :data-label="label" />'
					}
				}
			}
		})

		await wrapper.vm.$nextTick()
		expect(wrapper.find('.stub-json-editor').exists()).toBe(true)
	})

	it('should not log raw form payload (dataToValidate) to console.error on validation failure', async () => {
		const { vi } = await import('vitest')
		const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})

		const secretSchema = z.object({
			name: z.string().min(3),
			password: z.string().min(8)
		})

		const wrapper = mount(DynamicZodForm, {
			props: {
				schema: secretSchema,
				initialData: {
					name: 'a',
					password: 'super-secret-value-123'
				}
			}
		})

		await (wrapper.vm as any).validateAndSubmit()
		await wrapper.vm.$nextTick()

		expect(errorSpy).not.toHaveBeenCalled()
		const loggedText = errorSpy.mock.calls
			.flat()
			.map((arg) => JSON.stringify(arg))
			.join(' ')
		expect(loggedText).not.toContain('super-secret-value-123')

		errorSpy.mockRestore()
	})

	it('should sync external modelValue prop updates to form inputs without infinite loops', async () => {
		const simpleSchema = z.object({
			title: z.string()
		})

		const wrapper = mount(DynamicZodForm, {
			props: {
				schema: simpleSchema,
				modelValue: { title: 'initial' }
			}
		})

		const input = wrapper.find('input')
		expect((input.element as HTMLInputElement).value).toBe('initial')

		await wrapper.setProps({ modelValue: { title: 'external-update' } })
		await wrapper.vm.$nextTick()

		expect((input.element as HTMLInputElement).value).toBe('external-update')
	})

	it('should auto-initialize multi-branch union state with discriminatorDefault and branchDefaults when uninitialized', async () => {
		const multiUnionSchema = z.object({
			config: z.discriminatedUnion('type', [
				z.object({
					type: z.literal('alpha'),
					config: z.object({ host: z.string() })
				}),
				z.object({
					type: z.literal('beta'),
					config: z.object({ port: z.number() })
				})
			])
		})

		const model = ref<Record<string, any>>({})
		const wrapper = mount(DynamicZodForm, {
			props: {
				schema: multiUnionSchema,
				modelValue: model.value,
				'onUpdate:modelValue': (v: Record<string, any>) => {
					model.value = v
				},
				hints: {
					config: {
						discriminatorDefault: 'beta',
						branchDefaults: {
							beta: { port: 8080 }
						}
					}
				}
			}
		})

		await wrapper.vm.$nextTick()

		expect(model.value.config).toEqual({
			type: 'beta',
			config: { port: 8080 }
		})
	})

	it('should preserve null on multi-branch union when none option exists and value is null', async () => {
		const nullableUnionSchema = z.object({
			auth: z
				.union([
					z.null(),
					z.object({
						type: z.literal('basic'),
						username: z.string()
					}),
					z.object({
						type: z.literal('bearer'),
						token: z.string()
					})
				])
				.nullable()
		})

		const model = ref<Record<string, any>>({ auth: null })
		mount(DynamicZodForm, {
			props: {
				schema: nullableUnionSchema,
				modelValue: model.value,
				'onUpdate:modelValue': (v: Record<string, any>) => {
					model.value = v
				},
				hints: {
					auth: {
						discriminatorDefault: 'none'
					}
				}
			}
		})

		await Promise.resolve()
		expect(model.value.auth).toBeNull()
	})

	it('should apply branchDefaults when toggling optional object union checkbox on', async () => {
		const optUnionSchema = z.object({
			tls: z
				.union([
					z.null(),
					z.object({
						insecure: z.boolean(),
						ca_file: z.string().optional()
					})
				])
				.nullable()
		})

		const model = ref<Record<string, any>>({ tls: null })
		const wrapper = mount(DynamicZodForm, {
			props: {
				schema: optUnionSchema,
				modelValue: model.value,
				'onUpdate:modelValue': (v: Record<string, any>) => {
					model.value = v
				},
				hints: {
					tls: {
						branchDefaults: {
							enabled: { insecure: true }
						}
					}
				}
			}
		})

		await wrapper.vm.$nextTick()
		const checkbox = wrapper.findComponent({ name: 'Checkbox' })
		expect(checkbox.exists()).toBe(true)

		await checkbox.vm.$emit('update:checked', true)
		await wrapper.vm.$nextTick()

		expect(model.value.tls).toEqual({ insecure: true })
	})

	it('should not apply root #field:name slot to nested fields with leaf name "name"', async () => {
		const nestedSchema = z.object({
			name: z.string(),
			config: z.union([
				z.object({
					type: z.enum(['internal_logs']),
					config: z.object({
						include: z.array(
							z.object({
								name: z.string(),
								level: z.enum(['info', 'debug'])
							})
						)
					})
				})
			])
		})

		const wrapper = mount(DynamicZodForm, {
			props: {
				schema: nestedSchema,
				initialData: {
					name: 'Root Listener',
					config: {
						type: 'internal_logs',
						config: {
							include: [{ name: 'patrolli::core', level: 'debug' }]
						}
					}
				}
			},
			slots: {
				'field:name': (slotProps: any) =>
					h('div', { 'data-testid': 'root-name-slot' }, String(slotProps.value))
			},
			global: {
				stubs: {
					UiIcon: true,
					UiButton: true,
					AppInputSelect: true,
					AppInputText: {
						props: ['modelValue', 'label', 'placeholder'],
						template:
							'<input data-testid="nested-text-input" :value="modelValue" :data-label="label" :placeholder="placeholder" />'
					}
				}
			}
		})

		await wrapper.vm.$nextTick()

		const rootSlots = wrapper.findAll('[data-testid="root-name-slot"]')
		expect(rootSlots).toHaveLength(1)
		expect(rootSlots[0].text()).toBe('Root Listener')

		const nestedInputs = wrapper.findAll('[data-testid="nested-text-input"]')
		expect(nestedInputs).toHaveLength(1)
		expect((nestedInputs[0].element as HTMLInputElement).value).toBe('patrolli::core')
	})

	it('should resolve hints strictly by path, array-index normalization, and union aliases without leaf fallback', () => {
		const internalLogsRoot = { config: { type: 'internal_logs' } }
		const includeNameHint = resolveHintForPath(
			'config.config.include.0.name',
			listenerHints,
			internalLogsRoot
		)
		expect(includeNameHint?.label).toBe('Module Name')
		expect(includeNameHint?.placeholder).toBe('e.g. patrolli::pipeline')

		const unrelatedRootLeak = resolveHintForPath(
			'config.config.unknown_array.0.name',
			listenerHints,
			internalLogsRoot
		)
		expect(unrelatedRootLeak).toBeUndefined()

		const snmpRoot = { config: { type: 'snmp_trap' } }
		const snmpUsernameHint = resolveHintForPath(
			'config.config.v3.security_models.0.config.username',
			listenerHints,
			snmpRoot
		)
		expect(snmpUsernameHint?.label).toBe('Username / Security Name')

		const sinkDiskRoot = { channel: { type: 'disk' }, config: { type: 'file' } }
		const diskCompressionHint = resolveHintForPath(
			'channel.compression',
			sinkHints,
			sinkDiskRoot
		)
		expect(diskCompressionHint?.label).toBe('Compression Type')
	})
})

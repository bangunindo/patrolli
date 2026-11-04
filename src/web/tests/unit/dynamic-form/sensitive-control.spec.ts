// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import { z } from 'zod'
import {
	isNullableJsonSchema,
	isSensitiveJsonSchema,
	isSensitiveSchema,
	isSensitiveSetObject
} from '@/app/services/forms/sensitive.utils'
import { AutoFormAdapter } from '@/app/services/forms/auto-adapter'
import { sinkHints } from '@/app/services/forms/ui-hints/sink.hints'
import { Sensitive as OrvalSensitive } from '@@/shared/generated/patrolliAPI.schemas.type'
import DynamicZodForm from '@/components/app/dynamic-zod-form/DynamicZodForm.vue'
import Password from '@/components/app/input/Password.vue'

describe('Sensitive secret handling', () => {
	describe('Finding 1: sensitive.utils and nullable Orval unions', () => {
		it('should identify Sensitive Zod schema correctly including Orval nested nullable unions', () => {
			expect(isSensitiveSchema(OrvalSensitive)).toBe(true)

			const optionalSensitive = OrvalSensitive.optional()
			expect(isSensitiveSchema(optionalSensitive)).toBe(true)

			const nullableSensitive = OrvalSensitive.nullable()
			expect(isSensitiveSchema(nullableSensitive)).toBe(true)

			// Orval pattern: z.union([z.null(), OrvalSensitive]).optional()
			const orvalNullableSensitive = z.union([z.null(), OrvalSensitive]).optional()
			expect(isSensitiveSchema(orvalNullableSensitive)).toBe(true)

			// Non-sensitive schemas
			expect(isSensitiveSchema(z.string())).toBe(false)
			expect(isSensitiveSchema(z.union([z.string(), z.number()]))).toBe(false)
			expect(isSensitiveSchema(z.object({ set: z.string() }))).toBe(false)
			expect(isSensitiveSchema(null)).toBe(false)
		})

		it('should validate { set: "<handle>" } objects and reject empty handles', () => {
			expect(isSensitiveSetObject({ set: 'enc_handle_123' })).toBe(true)
			expect(isSensitiveSetObject({ set: 'a' })).toBe(true)

			// Objects with additional properties must be rejected
			expect(isSensitiveSetObject({ set: 'name', value: 'John Doe' })).toBe(false)
			expect(isSensitiveSetObject({ set: 'enc_handle_123', extra: true })).toBe(false)

			// Empty and whitespace handles must be rejected
			expect(isSensitiveSetObject({ set: '' })).toBe(false)
			expect(isSensitiveSetObject({ set: '   ' })).toBe(false)

			expect(isSensitiveSetObject('hunter2')).toBe(false)
			expect(isSensitiveSetObject(null)).toBe(false)
			expect(isSensitiveSetObject(undefined)).toBe(false)
			expect(isSensitiveSetObject({ not_set: 'handle' })).toBe(false)
			expect(isSensitiveSetObject({ set: 123 })).toBe(false)
		})

		it('should detect Sensitive in Draft-07 JSON Schema including nested nullable anyOf / oneOf', () => {
			const sensitiveJsonSchema = {
				anyOf: [
					{ type: 'string' },
					{
						type: 'object',
						properties: {
							set: { type: 'string' }
						},
						required: ['set']
					}
				]
			}
			expect(isSensitiveJsonSchema(sensitiveJsonSchema)).toBe(true)

			// Orval nested anyOf with null branch
			const orvalNestedJsonSchema = {
				anyOf: [
					{ type: 'null' },
					{
						anyOf: [
							{ type: 'string' },
							{
								type: 'object',
								properties: {
									set: { type: 'string' }
								}
							}
						]
					}
				]
			}
			expect(isSensitiveJsonSchema(orvalNestedJsonSchema)).toBe(true)
			expect(isNullableJsonSchema(orvalNestedJsonSchema)).toBe(true)

			const normalStringJsonSchema = { type: 'string' }
			expect(isSensitiveJsonSchema(normalStringJsonSchema)).toBe(false)
			expect(isNullableJsonSchema(normalStringJsonSchema)).toBe(false)

			const otherUnionJsonSchema = {
				anyOf: [{ type: 'string' }, { type: 'number' }]
			}
			expect(isSensitiveJsonSchema(otherUnionJsonSchema)).toBe(false)
		})
	})

	describe('Finding 2: Nested renderer and PasswordControl tester precedence', () => {
		it('should render PasswordControl for nested Sensitive fields without explicit UI hints', async () => {
			const nestedSchema = z.object({
				tls: z.object({
					key_password: z.union([z.null(), OrvalSensitive]).optional()
				})
			})

			const wrapper = mount(DynamicZodForm, {
				props: {
					schema: nestedSchema,
					initialData: {
						tls: {
							key_password: { set: 'enc_handle_nested' }
						}
					}
				},
				global: {
					stubs: {
						UiIcon: true,
						UiInput: true
					}
				}
			})

			await wrapper.vm.$nextTick()
			// Should render PasswordControl (Configured on server), NOT UnionControl select
			expect(wrapper.text()).toContain('Configured on server')
			expect(wrapper.text()).toContain('(Kept unchanged)')
			expect(wrapper.find('select').exists()).toBe(false)
		})
	})

	describe('Finding 3: AutoFormAdapter.toDraft and sink hints validation', () => {
		it('should initialize Sensitive fields properly in toDraft without emitting empty object {}', () => {
			const schema = z.object({
				req_pass: OrvalSensitive,
				opt_pass: z.union([z.null(), OrvalSensitive]).optional(),
				existing_pass: OrvalSensitive
			})

			const entity = {
				opt_pass: null,
				existing_pass: { set: 'enc_saved_handle' }
			}

			const draft = AutoFormAdapter.toDraft(schema, entity)
			expect(draft.req_pass).toBe('')
			expect(draft.opt_pass).toBeNull()
			expect(draft.existing_pass).toEqual({ set: 'enc_saved_handle' })
		})

		it('should preserve non-sensitive objects having set property in toDraft and toRequest', () => {
			const nonSensitiveSchema = z.object({
				filter: z.object({
					set: z.string(),
					value: z.string()
				})
			})

			const entity = {
				filter: { set: 'name', value: 'John Doe' }
			}

			// 1. toDraft must not strip value property
			const draft = AutoFormAdapter.toDraft(nonSensitiveSchema, entity)
			expect(draft.filter).toEqual({ set: 'name', value: 'John Doe' })

			// 2. toRequest (deepSanitize) must not strip value property
			const result = AutoFormAdapter.toRequest(nonSensitiveSchema, draft)
			expect(result.success).toBe(true)
			if (result.success) {
				expect(result.data.filter).toEqual({ set: 'name', value: 'John Doe' })
			}
		})

		it('should validate SASL password in sink hints rejecting empty object and accepting handle', () => {
			const saslValidator = sinkHints['config.kafka.sasl.password']?.validator
			expect(saslValidator).toBeDefined()

			const draftWithSasl = {
				config: {
					type: 'kafka',
					config: {
						security: { protocol: 'sasl_plaintext' }
					}
				}
			}

			// Valid plaintext password
			expect(saslValidator!('secret123', draftWithSasl)).toBeUndefined()
			// Valid sensitive handle
			expect(saslValidator!({ set: 'enc_handle_pass' }, draftWithSasl)).toBeUndefined()
			// Empty string rejected
			expect(saslValidator!('', draftWithSasl)).toBe('Password is required for SASL protocol')
			// Empty object rejected
			expect(saslValidator!({}, draftWithSasl)).toBe('Password is required for SASL protocol')
			// Empty handle rejected
			expect(saslValidator!({ set: '' }, draftWithSasl)).toBe(
				'Password is required for SASL protocol'
			)
		})
	})

	describe('Finding 4: PasswordControl state transitions and empty change validation', () => {
		it('should render Configured mode when initial data is { set: "<handle>" }', async () => {
			const schema = z.object({
				password: OrvalSensitive
			})

			const wrapper = mount(DynamicZodForm, {
				props: {
					schema,
					initialData: {
						password: { set: 'test-nonce-handle-123' }
					}
				},
				global: {
					stubs: {
						UiIcon: true
					}
				}
			})

			await wrapper.vm.$nextTick()
			expect(wrapper.text()).toContain('Configured on server')
			expect(wrapper.text()).toContain('(Kept unchanged)')
			expect(wrapper.text()).toContain('Change')
		})

		it('should switch to editing mode on clicking Change and allow reverting', async () => {
			const schema = z.object({
				password: OrvalSensitive
			})

			const wrapper = mount(DynamicZodForm, {
				props: {
					schema,
					initialData: {
						password: { set: 'test-nonce-handle-123' }
					}
				},
				global: {
					stubs: {
						UiIcon: true
					}
				}
			})

			await wrapper.vm.$nextTick()
			expect(wrapper.text()).toContain('Configured on server')

			// Click Change button
			const changeBtn = wrapper.findAll('button').find((b) => b.text().includes('Change'))!
			await changeBtn.trigger('click')
			await wrapper.vm.$nextTick()

			// Now in editing mode
			expect(wrapper.text()).toContain('Changing secret...')
			expect(wrapper.text()).toContain('Revert')

			// Click Revert button
			const revertBtn = wrapper.findAll('button').find((b) => b.text().includes('Revert'))!
			await revertBtn.trigger('click')
			await wrapper.vm.$nextTick()

			// Returns to Configured mode
			expect(wrapper.text()).toContain('Configured on server')
		})

		it('should preserve savedSetHandle and Changing secret state while typing new secret', async () => {
			const schema = z.object({
				password: OrvalSensitive
			})

			const wrapper = mount(DynamicZodForm, {
				props: {
					schema,
					initialData: {
						password: { set: 'test-nonce-handle-123' }
					}
				},
				global: {
					stubs: {
						UiIcon: true
					}
				}
			})

			await wrapper.vm.$nextTick()
			const changeBtn = wrapper.findAll('button').find((b) => b.text().includes('Change'))!
			await changeBtn.trigger('click')
			await wrapper.vm.$nextTick()

			expect(wrapper.text()).toContain('Changing secret...')

			// Simulate typing sequentially via Password component modelValue updates
			const passwordComp = wrapper.findComponent(Password)
			expect(passwordComp.exists()).toBe(true)

			await passwordComp.vm.$emit('update:modelValue', 'p')
			await wrapper.vm.$nextTick()
			expect(wrapper.text()).toContain('Changing secret...')
			expect(wrapper.text()).toContain('Revert')

			await passwordComp.vm.$emit('update:modelValue', 'pass123')
			await wrapper.vm.$nextTick()
			expect(wrapper.text()).toContain('Changing secret...')
			expect(wrapper.text()).toContain('Revert')

			// Revert should still work after typing
			const revertBtn = wrapper.findAll('button').find((b) => b.text().includes('Revert'))!
			await revertBtn.trigger('click')
			await wrapper.vm.$nextTick()
			expect(wrapper.text()).toContain('Configured on server')
		})

		it('should validate and block submission when Change is clicked and left empty', async () => {
			const schema = z.object({
				password: OrvalSensitive
			})

			const wrapper = mount(DynamicZodForm, {
				props: {
					schema,
					initialData: {
						password: { set: 'test-nonce-handle-123' }
					}
				},
				global: {
					stubs: {
						UiIcon: true
					}
				}
			})

			await wrapper.vm.$nextTick()
			const changeBtn = wrapper.findAll('button').find((b) => b.text().includes('Change'))!
			await changeBtn.trigger('click')
			await wrapper.vm.$nextTick()

			// Submit the form while input is empty
			const form = wrapper.find('form')
			await form.trigger('submit')
			await wrapper.vm.$nextTick()

			// Should display validation error explaining user choices
			expect(wrapper.text()).toContain(
				'New secret cannot be empty; enter a new value, click Revert, or click Remove'
			)
			expect(wrapper.emitted('submit')).toBeUndefined()
		})

		it('should restore Configured mode on form reset after clicking Change', async () => {
			const schema = z.object({
				password: OrvalSensitive
			})

			const wrapper = mount(DynamicZodForm, {
				props: {
					schema,
					initialData: {
						password: { set: 'test-nonce-handle-123' }
					}
				},
				global: {
					stubs: {
						UiIcon: true
					}
				}
			})

			await wrapper.vm.$nextTick()
			const changeBtn = wrapper.findAll('button').find((b) => b.text().includes('Change'))!
			await changeBtn.trigger('click')
			await wrapper.vm.$nextTick()
			expect(wrapper.text()).toContain('Changing secret...')

			// Call reset() exposed method
			;(wrapper.vm as any).reset()
			await wrapper.vm.$nextTick()

			// Expect mode to return to Configured
			expect(wrapper.text()).toContain('Configured on server')
		})

		it('should allow removing optional secret and undoing the removal', async () => {
			const schema = z.object({
				tls_password: OrvalSensitive.optional()
			})

			const wrapper = mount(DynamicZodForm, {
				props: {
					schema,
					initialData: {
						tls_password: { set: 'test-nonce-handle-123' }
					}
				},
				global: {
					stubs: {
						UiIcon: true,
						UiInput: true
					}
				}
			})

			await wrapper.vm.$nextTick()
			const removeBtn = wrapper.findAll('button').find((b) => b.text().includes('Remove'))!
			expect(removeBtn).toBeDefined()
			await removeBtn.trigger('click')
			await wrapper.vm.$nextTick()

			// In removed mode
			expect(wrapper.text()).toContain('Secret removed')
			expect(wrapper.text()).toContain('(Will be cleared on save)')

			// Click Undo
			const undoBtn = wrapper.findAll('button').find((b) => b.text().includes('Undo'))!
			await undoBtn.trigger('click')
			await wrapper.vm.$nextTick()

			// Back to configured
			expect(wrapper.text()).toContain('Configured on server')
		})
	})
})

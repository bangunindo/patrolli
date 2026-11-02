// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { defineComponent, ref } from 'vue'
import { z } from 'zod'
import { useValidation } from '@/composables/useValidation'
import { validateRules } from '@/app/services/forms/format-registry'
import AppInputError from '@/components/app/input/AppInputError.vue'
import AppInputText from '@/components/app/input/AppInputText.vue'
import AppInputSelect from '@/components/app/input/AppInputSelect.vue'
import Password from '@/components/app/input/Password.vue'
import ArrayString from '@/components/app/input/ArrayString.vue'
import AppInputHost from '@/components/app/input/AppInputHost.vue'
import AppInputSelectTable from '@/components/app/input/AppInputSelectTable.vue'
import AppInputLabel from '@/components/app/input/Label.vue'
import AppInputAttributeFilter from '@/components/app/input/AttributeFilter.vue'

describe('Centralized Validation Engine & Standardized UI Error Presentation', () => {
	const pinia = createPinia()
	setActivePinia(pinia)
	describe('useValidation Composable Core Capabilities', () => {
		it('should automatically integrate format registry rules', () => {
			const val = ref('invalid-email')
			const rules = ref(['required', 'email'])
			const { validate, messages, hasError, isValid } = useValidation(
				'User Email',
				val,
				rules
			)

			const errs = validate()
			expect(errs).toContain('User Email field must be a valid email address')
			expect(hasError.value).toBe(true)
			expect(isValid.value).toBe(false)
			expect(messages.value).toEqual(errs)

			val.value = 'admin@patrolli.io'
			const validErrs = validate()
			expect(validErrs).toEqual([])
			expect(hasError.value).toBe(false)
			expect(isValid.value).toBe(true)
		})

		it('should validate array items correctly even if first item is valid and second item is invalid', () => {
			const context = { path: ['ips'], pathKey: 'ips', label: 'IP List' }
			const errs = validateRules(['192.168.1.1', 'not-an-ip'], ['ipv4'], context)
			expect(errs).toContain('IP List field must be a valid IPv4 address (e.g. 192.168.1.1)')
		})

		it('should support direct validation with optional Zod schema', () => {
			const val = ref(15)
			const AgeSchema = z.number().int().min(18, 'Minimum age is 18 years')
			const { validate, hasError, errorMessage } = useValidation({
				label: 'Age',
				value: val,
				schema: AgeSchema
			})

			const errs = validate()
			expect(errs.length).toBeGreaterThan(0)
			expect(hasError.value).toBe(true)
			expect(errorMessage.value).toMatch(/18/)
		})

		it('should support syncing external error prop from DynamicForm or API', async () => {
			const val = ref('sample')
			const externalError = ref('Host name already in use')
			const onErrorChange = vi.fn()

			const { hasError, errorMessage, showMessages } = useValidation('Host Name', val, [], {
				error: externalError,
				onErrorChange
			})

			expect(hasError.value).toBe(true)
			expect(showMessages.value).toBe(true)
			expect(errorMessage.value).toBe('Host name already in use')

			// User types / modifies input value
			val.value = 'sample-renamed'
			await new Promise((r) => setTimeout(r, 10))

			// Ensure onErrorChange callback is called to clear error in parent
			expect(onErrorChange).toHaveBeenCalledWith('')
		})

		it('should set and clear manual error messages with setError and clearError', () => {
			const val = ref('')
			const { setError, clearError, messages, hasError, errorMessage } = useValidation(
				'Token',
				val,
				[]
			)

			setError('Token expired')
			expect(hasError.value).toBe(true)
			expect(errorMessage.value).toBe('Token expired')
			expect(messages.value).toEqual(['Token expired'])

			clearError()
			expect(hasError.value).toBe(false)
			expect(messages.value).toEqual([])
		})

		it('should close alert and reset state when handleAlertClose is called', () => {
			const val = ref('')
			const onClose = vi.fn()
			const { setError, handleAlertClose, showMessages, hasError, messages } = useValidation(
				'Code',
				val,
				[],
				{ onClose }
			)

			setError('Invalid code')
			expect(showMessages.value).toBe(true)
			expect(hasError.value).toBe(true)

			handleAlertClose()
			expect(showMessages.value).toBe(false)
			expect(messages.value).toEqual([])
			expect(onClose).toHaveBeenCalled()
		})

		it('should re-trigger alert when validate is called again after alert was closed', () => {
			const val = ref('')
			const { validate, handleAlertClose, showMessages, hasError, messages } = useValidation(
				'Code',
				val,
				['required']
			)

			validate()
			expect(showMessages.value).toBe(true)
			expect(hasError.value).toBe(true)
			expect(messages.value.length).toBeGreaterThan(0)

			handleAlertClose()
			expect(showMessages.value).toBe(false)
			expect(messages.value).toEqual([])

			// Re-validate / Re-submit
			validate()
			expect(showMessages.value).toBe(true)
			expect(hasError.value).toBe(true)
			expect(messages.value.length).toBeGreaterThan(0)
		})
	})

	describe('AppInputError Standardized UI Component', () => {
		it('should render a single string as title in b-alert', () => {
			const wrapper = mount(AppInputError, {
				props: {
					messages: 'This field is required'
				},
				global: {
					stubs: {
						'b-alert': {
							props: ['title', 'type', 'shape', 'closeable'],
							template: '<div class="stubbed-alert">{{ title }}</div>'
						}
					}
				}
			})

			expect(wrapper.text()).toContain('This field is required')
		})

		it('should join array of error messages with pipe separator ( | )', () => {
			const wrapper = mount(AppInputError, {
				props: {
					messages: ['Error 1', 'Error 2']
				},
				global: {
					stubs: {
						'b-alert': {
							props: ['title'],
							template: '<div class="stubbed-alert">{{ title }}</div>'
						}
					}
				}
			})

			expect(wrapper.text()).toContain('Error 1 | Error 2')
		})

		it('should not render anything when there is no error', () => {
			const wrapper = mount(AppInputError, {
				props: {
					messages: []
				},
				global: {
					stubs: {
						'b-alert': true
					}
				}
			})

			expect(wrapper.find('div').exists()).toBe(false)
		})

		it('should support uncontrolled dismissal and reactivate when a new error arrives', async () => {
			const wrapper = mount(AppInputError, {
				props: {
					messages: 'Invalid date format'
				}
			})

			expect(wrapper.find('div').exists()).toBe(true)

			// Click close button on alert
			await wrapper.find('button').trigger('click')
			expect(wrapper.find('div').exists()).toBe(false)

			// New error arrives from form validator
			await wrapper.setProps({ messages: 'Required' })
			expect(wrapper.find('div').exists()).toBe(true)
			expect(wrapper.text()).toContain('Required')
		})

		it('should reset isDismissed and redisplay alert when modelValue is reactivated by parent', async () => {
			const wrapper = mount(AppInputError, {
				props: {
					modelValue: true,
					messages: 'This field is required'
				}
			})

			expect(wrapper.find('div').exists()).toBe(true)

			// Close alert
			await wrapper.find('button').trigger('click')
			expect(wrapper.emitted('update:modelValue')?.[0]).toEqual([false])

			await wrapper.setProps({ modelValue: false })
			expect(wrapper.find('div').exists()).toBe(false)

			// Parent reactivates modelValue (e.g. on resubmit without changing error text)
			await wrapper.setProps({ modelValue: true })
			expect(wrapper.find('div').exists()).toBe(true)
		})
	})

	describe('Atomic & Complex Input Component Integration', () => {
		it('should display consistent error in AppInputText when error prop is provided', async () => {
			const wrapper = mount(AppInputText, {
				props: {
					label: 'Endpoint Name',
					error: 'Duplicate endpoint name'
				},
				global: {
					stubs: {
						'b-input': true,
						'b-alert': {
							props: ['title'],
							template: '<div class="alert-mock">{{ title }}</div>'
						}
					}
				}
			})

			expect(wrapper.text()).toContain('Duplicate endpoint name')
		})

		it('should display consistent error in AppInputSelect when error prop is provided', async () => {
			const wrapper = mount(AppInputSelect, {
				props: {
					modelValue: null,
					label: 'Cluster Node',
					options: [{ label: 'Node 1', value: 'node-1' }],
					error: 'Select at least one node'
				},
				global: {
					stubs: {
						'b-alert': {
							props: ['title'],
							template: '<div class="alert-mock">{{ title }}</div>'
						}
					}
				}
			})

			expect(wrapper.text()).toContain('Select at least one node')
		})

		it('should display consistent error in Password component when error prop is provided', async () => {
			const wrapper = mount(Password, {
				props: {
					error: 'Password too weak'
				},
				global: {
					stubs: {
						'b-input': true,
						'b-alert': {
							props: ['title'],
							template: '<div class="alert-mock">{{ title }}</div>'
						}
					}
				}
			})

			expect(wrapper.text()).toContain('Password too weak')
		})

		it('should display consistent error in ArrayString component when error prop is provided', async () => {
			const wrapper = mount(ArrayString, {
				props: {
					label: 'Tags',
					error: 'Tags cannot be empty'
				},
				global: {
					stubs: {
						'b-button': true,
						'b-link': true,
						'b-icon': true,
						'b-text': true,
						'app-input-text': true,
						'b-alert': {
							props: ['title'],
							template: '<div class="alert-mock">{{ title }}</div>'
						}
					}
				}
			})

			expect(wrapper.text()).toContain('Tags cannot be empty')
		})

		it('should forward error prop to AppInputSelectTable and render error banner in AppInputHost', async () => {
			const wrapper = mount(AppInputHost, {
				props: {
					label: 'Target Host',
					error: 'Host is required'
				},
				global: {
					plugins: [pinia],
					stubs: {
						'b-modal': true,
						'b-card': true,
						'b-input': true,
						'b-button': true,
						'b-icon': true,
						'b-alert': {
							props: ['title'],
							template: '<div class="alert-mock">{{ title }}</div>'
						}
					}
				}
			})

			expect(wrapper.text()).toContain('Host is required')
		})

		it('should forward disabled prop to AppInputSelectTable and prevent interaction in AppInputHost', async () => {
			const wrapper = mount(AppInputHost, {
				props: {
					label: 'Target Host',
					disabled: true
				},
				global: {
					plugins: [pinia],
					stubs: {
						'b-modal': true,
						'b-card': true,
						'b-input': true,
						'b-button': true,
						'b-icon': true,
						UiDialog: true
					}
				}
			})

			const selectTable = wrapper.findComponent(AppInputSelectTable)
			expect(selectTable.exists()).toBe(true)
			expect(selectTable.props('disabled')).toBe(true)

			const triggerDiv = selectTable.find('div[tabindex="-1"]')
			expect(triggerDiv.exists()).toBe(true)
			expect(triggerDiv.classes()).toContain('cursor-not-allowed')
		})

		it('should support error prop and useValidation in Label.vue', async () => {
			const wrapper = mount(AppInputLabel, {
				props: {
					modelValue: '{}',
					label: 'Service Labels',
					error: 'Labels must have at least 1 key-value pair'
				},
				global: {
					stubs: {
						'b-input': true,
						'b-button': true,
						'b-icon': true,
						'b-link': true,
						'b-text': true,
						'b-alert': {
							props: ['title'],
							template: '<div class="alert-mock">{{ title }}</div>'
						}
					}
				}
			})

			expect(wrapper.text()).toContain('Labels must have at least 1 key-value pair')
		})

		it('should render AppInputError in AppInputAttributeFilter when root is true and error is present', async () => {
			const wrapper = mount(AppInputAttributeFilter, {
				props: {
					modelValue: {
						match: {
							key: 'env',
							key_is_regex: false,
							op: '=',
							table: 'record',
							value: { string: 'prod' }
						}
					},
					error: 'Filter rule is required',
					root: true
				},
				global: {
					stubs: {
						AppInputSelect: true,
						AppInputText: true,
						'b-button': true,
						'b-switch': true,
						'b-alert': {
							props: ['title'],
							template: '<div class="alert-mock">{{ title }}</div>'
						}
					}
				}
			})

			expect(wrapper.find('[data-invalid="true"]').exists()).toBe(true)
			expect(wrapper.classes()).toContain('has-error')
			expect(wrapper.text()).toContain('Filter rule is required')
		})

		it('should mark all invalid inputs with error status on submit in classic forms', async () => {
			const FormComp = defineComponent({
				components: { AppInputText },
				template: `
					<form @submit.prevent>
						<AppInputText ref="fieldA" label="First Name" required />
						<AppInputText ref="fieldB" label="Last Name" required />
						<button type="submit" id="submit-btn">Submit</button>
					</form>
				`
			})

			const wrapper = mount(FormComp, {
				attachTo: document.body
			})

			await wrapper.find('form').trigger('submit')
			await new Promise((r) => setTimeout(r, 50))

			const alerts = wrapper.findAllComponents(AppInputError)
			expect(alerts.filter((a) => a.find('div').exists()).length).toBe(2)
			expect(wrapper.text()).toContain('First Name field is required')
			expect(wrapper.text()).toContain('Last Name field is required')

			wrapper.unmount()
		})

		it('should reappear alert when form is resubmitted after field alert was closed', async () => {
			const FormComp = defineComponent({
				components: { AppInputText },
				template: `
					<form @submit.prevent>
						<AppInputText ref="fieldA" label="First Name" required />
						<button type="submit" id="submit-btn">Submit</button>
					</form>
				`
			})

			const wrapper = mount(FormComp, {
				attachTo: document.body
			})

			// 1. First submit -> error appears
			await wrapper.find('form').trigger('submit')
			await new Promise((r) => setTimeout(r, 50))
			const errorComp = wrapper.findComponent(AppInputError)
			expect(errorComp.find('div').exists()).toBe(true)

			// 2. Click close on field error alert
			await errorComp.find('button').trigger('click')
			await wrapper.vm.$nextTick()
			expect(errorComp.find('div').exists()).toBe(false)

			// 3. Resubmit form without changing input -> error alert reappears
			await wrapper.find('form').trigger('submit')
			await new Promise((r) => setTimeout(r, 50))
			expect(errorComp.find('div').exists()).toBe(true)
			expect(errorComp.text()).toContain('First Name field is required')

			wrapper.unmount()
		})
	})
})

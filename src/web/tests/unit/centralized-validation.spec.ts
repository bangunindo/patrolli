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
		it('mengintegrasikan format registry rules secara otomatis', () => {
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

		it('memvalidasi format item array dengan benar meskipun item pertama valid dan item kedua tidak valid', () => {
			const context = { path: ['ips'], pathKey: 'ips', label: 'IP List' }
			const errs = validateRules(['192.168.1.1', 'not-an-ip'], ['ipv4'], context)
			expect(errs).toContain('IP List field must be a valid IPv4 address (e.g. 192.168.1.1)')
		})

		it('mendukung validasi langsung dengan Zod Schema opsional', () => {
			const val = ref(15)
			const AgeSchema = z.number().int().min(18, 'Minimal usia adalah 18 tahun')
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

		it('mendukung sinkronisasi external error prop dari DynamicForm atau API', async () => {
			const val = ref('sample')
			const externalError = ref('Nama host sudah digunakan')
			const onErrorChange = vi.fn()

			const { hasError, errorMessage, showMessages } = useValidation('Host Name', val, [], {
				error: externalError,
				onErrorChange
			})

			expect(hasError.value).toBe(true)
			expect(showMessages.value).toBe(true)
			expect(errorMessage.value).toBe('Nama host sudah digunakan')

			// User types / modifies input value
			val.value = 'sample-renamed'
			await new Promise((r) => setTimeout(r, 10))

			// Ensure onErrorChange callback is called to clear error in parent
			expect(onErrorChange).toHaveBeenCalledWith('')
		})

		it('dapat mengatur dan membersihkan pesan error manual dengan setError dan clearError', () => {
			const val = ref('')
			const { setError, clearError, messages, hasError, errorMessage } = useValidation(
				'Token',
				val,
				[]
			)

			setError('Token kedaluwarsa')
			expect(hasError.value).toBe(true)
			expect(errorMessage.value).toBe('Token kedaluwarsa')
			expect(messages.value).toEqual(['Token kedaluwarsa'])

			clearError()
			expect(hasError.value).toBe(false)
			expect(messages.value).toEqual([])
		})

		it('menutup alert dan mereset state saat handleAlertClose dipanggil', () => {
			const val = ref('')
			const onClose = vi.fn()
			const { setError, handleAlertClose, showMessages, hasError, messages } = useValidation(
				'Code',
				val,
				[],
				{ onClose }
			)

			setError('Kode salah')
			expect(showMessages.value).toBe(true)
			expect(hasError.value).toBe(true)

			handleAlertClose()
			expect(showMessages.value).toBe(false)
			expect(messages.value).toEqual([])
			expect(onClose).toHaveBeenCalled()
		})

		it('memunculkan kembali alert saat validate dipanggil ulang setelah alert ditutup', () => {
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
		it('merender string tunggal sebagai title pada b-alert', () => {
			const wrapper = mount(AppInputError, {
				props: {
					messages: 'Field ini wajib diisi'
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

			expect(wrapper.text()).toContain('Field ini wajib diisi')
		})

		it('menggabungkan array pesan error dengan pemisah pipe ( | )', () => {
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

		it('tidak merender apapun jika tidak ada error', () => {
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

		it('mendukung dismissal (close) tanpa v-model (uncontrolled) dan re-aktif saat error baru datang', async () => {
			const wrapper = mount(AppInputError, {
				props: {
					messages: 'Format tanggal salah'
				}
			})

			expect(wrapper.find('div').exists()).toBe(true)

			// Click close button on alert
			await wrapper.find('button').trigger('click')
			expect(wrapper.find('div').exists()).toBe(false)

			// New error arrives from form validator
			await wrapper.setProps({ messages: 'Wajib diisi' })
			expect(wrapper.find('div').exists()).toBe(true)
			expect(wrapper.text()).toContain('Wajib diisi')
		})

		it('mereset isDismissed dan menampilkan kembali alert saat modelValue diaktifkan kembali oleh parent', async () => {
			const wrapper = mount(AppInputError, {
				props: {
					modelValue: true,
					messages: 'Field ini wajib diisi'
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
		it('AppInputText menampilkan error konsisten saat menerima prop error', async () => {
			const wrapper = mount(AppInputText, {
				props: {
					label: 'Endpoint Name',
					error: 'Nama endpoint duplikat'
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

			expect(wrapper.text()).toContain('Nama endpoint duplikat')
		})

		it('AppInputSelect menampilkan error konsisten saat menerima prop error', async () => {
			const wrapper = mount(AppInputSelect, {
				props: {
					modelValue: null,
					label: 'Cluster Node',
					options: [{ label: 'Node 1', value: 'node-1' }],
					error: 'Pilih minimal satu node'
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

			expect(wrapper.text()).toContain('Pilih minimal satu node')
		})

		it('Password component menampilkan error konsisten saat menerima prop error', async () => {
			const wrapper = mount(Password, {
				props: {
					error: 'Password terlalu lemah'
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

			expect(wrapper.text()).toContain('Password terlalu lemah')
		})

		it('ArrayString component menampilkan error konsisten saat menerima prop error', async () => {
			const wrapper = mount(ArrayString, {
				props: {
					label: 'Tags',
					error: 'Tags tidak boleh kosong'
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

			expect(wrapper.text()).toContain('Tags tidak boleh kosong')
		})

		it('AppInputHost meneruskan prop error ke AppInputSelectTable dan merender banner error', async () => {
			const wrapper = mount(AppInputHost, {
				props: {
					label: 'Target Host',
					error: 'Host wajib dipilih'
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

			expect(wrapper.text()).toContain('Host wajib dipilih')
		})

		it('AppInputHost meneruskan prop disabled ke AppInputSelectTable dan mencegah interaksi', async () => {
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

		it('Label.vue mendukung error prop dan useValidation', async () => {
			const wrapper = mount(AppInputLabel, {
				props: {
					modelValue: '{}',
					label: 'Service Labels',
					error: 'Labels harus memiliki minimal 1 pasangan key-value'
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

			expect(wrapper.text()).toContain('Labels harus memiliki minimal 1 pasangan key-value')
		})

		it('AppInputAttributeFilter merender AppInputError saat root dan ada error', async () => {
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
					error: 'Aturan filter wajib diisi',
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
			expect(wrapper.text()).toContain('Aturan filter wajib diisi')
		})

		it('semua input dalam classic form yang tidak valid menerima status error pada submit', async () => {
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

		it('memunculkan kembali alert saat form di-submit ulang setelah alert field ditutup', async () => {
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

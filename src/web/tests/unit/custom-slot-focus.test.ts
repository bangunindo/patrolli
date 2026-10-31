// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import { defineComponent, h, nextTick, ref } from 'vue'
import { z } from 'zod'
import CustomSlotRenderer from '@/components/app/dynamic-zod-form/renderers/CustomSlotRenderer.vue'
import DynamicZodForm from '@/components/app/dynamic-zod-form/DynamicZodForm.vue'

describe('CustomSlotRenderer DOM stability and focus preservation', () => {
	it('mempertahankan elemen DOM input saat parent me-render ulang slot function closure baru', async () => {
		const ParentComponent = defineComponent({
			setup() {
				const count = ref(0)
				const text = ref('initial')

				return { count, text }
			},
			render() {
				// Slot closures are recreated on each parent render
				const currentSlotFn = (props: any) =>
					h('input', {
						class: 'test-slot-input',
						value: props.value,
						onInput: (e: any) => {
							this.text = e.target.value
						}
					})

				return h('div', [
					h('button', {
						id: 'bump-btn',
						onClick: () => {
							this.count++
						}
					}),
					h(CustomSlotRenderer, {
						renderFn: currentSlotFn,
						slotProps: { value: this.text, count: this.count }
					})
				])
			}
		})

		const wrapper = mount(ParentComponent, {
			attachTo: document.body
		})
		const inputBefore = wrapper.find('.test-slot-input').element as HTMLInputElement

		// Set focus on input
		inputBefore.focus()
		expect(document.activeElement).toBe(inputBefore)

		// Trigger parent re-render (which generates a new closure for currentSlotFn)
		await wrapper.find('#bump-btn').trigger('click')
		await nextTick()

		const inputAfter = wrapper.find('.test-slot-input').element as HTMLInputElement

		// Node identity must be identical (not unmounted and remounted)
		expect(inputAfter).toBe(inputBefore)
		expect(document.activeElement).toBe(inputAfter)

		wrapper.unmount()
	})

	it('mempertahankan elemen input pada field slot saat DynamicZodForm menerima input data', async () => {
		const testSchema = z.object({
			name: z.string(),
			description: z.string().optional()
		})

		const HostComponent = defineComponent({
			components: { DynamicZodForm },
			setup() {
				const formData = ref({
					name: 'Proc',
					description: 'Desc'
				})
				return { formData, testSchema }
			},
			template: `
				<div>
					<DynamicZodForm
						v-model="formData"
						:schema="testSchema"
					>
						<template #field:name="{ modelValue, updateModelValue }">
							<input
								id="custom-name-field"
								:value="modelValue"
								@input="updateModelValue($event.target.value)"
							/>
						</template>
					</DynamicZodForm>
				</div>
			`
		})

		const wrapper = mount(HostComponent, {
			attachTo: document.body
		})

		await nextTick()

		const inputBefore = wrapper.find('#custom-name-field').element as HTMLInputElement
		expect(inputBefore).toBeTruthy()
		expect(inputBefore.value).toBe('Proc')

		// Focus the custom slot input
		inputBefore.focus()
		expect(document.activeElement).toBe(inputBefore)

		// Simulate user typing character by character
		await wrapper.find('#custom-name-field').setValue('Proce')
		await nextTick()

		const inputAfter = wrapper.find('#custom-name-field').element as HTMLInputElement

		// Element reference and focus must be preserved
		expect(inputAfter).toBe(inputBefore)
		expect(document.activeElement).toBe(inputAfter)
		expect(wrapper.vm.formData.name).toBe('Proce')

		wrapper.unmount()
	})
})

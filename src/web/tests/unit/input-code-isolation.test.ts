// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import AppInputCode from '@/components/app/input/AppInputCode.vue'
import JsonEditor from '@/components/app/input/Json.vue'

vi.mock('@/composables/useMonaco', () => ({
	useMonaco: () => ({
		loadMonaco: vi.fn().mockResolvedValue({
			Uri: { parse: (s: string) => s },
			editor: {
				create: vi.fn().mockReturnValue({
					getValue: vi.fn().mockReturnValue(''),
					getModel: vi.fn().mockReturnValue(null),
					dispose: vi.fn(),
					onDidChangeModelContent: vi.fn().mockReturnValue({ dispose: vi.fn() }),
					updateOptions: vi.fn()
				}),
				createModel: vi.fn().mockReturnValue({ dispose: vi.fn() }),
				getModel: vi.fn().mockReturnValue(null),
				setTheme: vi.fn(),
				setModelLanguage: vi.fn()
			},
			languages: {
				getLanguages: vi.fn().mockReturnValue([]),
				register: vi.fn(),
				setLanguageConfiguration: vi.fn(),
				setMonarchTokensProvider: vi.fn(),
				registerCompletionItemProvider: vi.fn()
			}
		}),
		registerJsonSchema: vi.fn(),
		unregisterJsonSchema: vi.fn()
	})
}))

describe('Monaco Editor VueFlow Keyboard Isolation', () => {
	beforeEach(() => {
		const pinia = createPinia()
		setActivePinia(pinia)
	})

	it('AppInputCode stops keydown event propagation on editor container', async () => {
		const pinia = createPinia()
		const wrapper = mount(AppInputCode, {
			props: {
				modelValue: 'logs | where alert == true',
				language: 'opl'
			},
			global: {
				plugins: [pinia]
			}
		})

		const editorInstance = wrapper.find('.editor-instance')
		const event = new KeyboardEvent('keydown', {
			key: ' ',
			code: 'Space',
			bubbles: true,
			cancelable: true
		})
		const stopPropagationSpy = vi.spyOn(event, 'stopPropagation')

		editorInstance.element.dispatchEvent(event)

		expect(stopPropagationSpy).toHaveBeenCalled()
	})

	it('Json component stops keydown event propagation on editor container', async () => {
		const pinia = createPinia()
		const wrapper = mount(JsonEditor, {
			props: {
				modelValue: { key: 'value' }
			},
			global: {
				plugins: [pinia]
			}
		})

		const editorInstance = wrapper.find('.editor-instance')
		const event = new KeyboardEvent('keydown', {
			key: ' ',
			code: 'Space',
			bubbles: true,
			cancelable: true
		})
		const stopPropagationSpy = vi.spyOn(event, 'stopPropagation')

		editorInstance.element.dispatchEvent(event)

		expect(stopPropagationSpy).toHaveBeenCalled()
	})
})

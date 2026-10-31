// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import {
	oplLanguageConfiguration,
	oplMonarchTokensProvider,
	registerOplLanguage
} from '@/app/helpers/opl.language'
import { describe, expect, it, vi } from 'vitest'

describe('opl.language', () => {
	it('should register language, configuration, and completion provider on monaco instance', () => {
		const registeredLanguages: any[] = []
		const configurations: Record<string, any> = {}
		const tokenProviders: Record<string, any> = {}
		const completionProviders: Record<string, any> = {}

		const mockMonaco: any = {
			languages: {
				getLanguages: () => registeredLanguages,
				register: vi.fn((def) => registeredLanguages.push(def)),
				setLanguageConfiguration: vi.fn((id, config) => {
					configurations[id] = config
				}),
				setMonarchTokensProvider: vi.fn((id, provider) => {
					tokenProviders[id] = provider
				}),
				registerCompletionItemProvider: vi.fn((id, provider) => {
					completionProviders[id] = provider
				}),
				CompletionItemKind: {
					Keyword: 1,
					Snippet: 27
				},
				CompletionItemInsertTextRule: {
					InsertAsSnippet: 4
				}
			}
		}

		registerOplLanguage(mockMonaco)

		expect(mockMonaco.languages.register).toHaveBeenCalledWith(
			expect.objectContaining({ id: 'opl' })
		)
		expect(configurations['opl']).toBe(oplLanguageConfiguration)
		expect(tokenProviders['opl']).toBe(oplMonarchTokensProvider)
		expect(completionProviders['opl']).toBeDefined()

		// Verify completions
		const mockModel = {
			getWordUntilPosition: () => ({ startColumn: 1, endColumn: 1 })
		}
		const completions = completionProviders['opl'].provideCompletionItems(mockModel, {
			lineNumber: 1
		})
		const labels = completions.suggestions.map((s: any) => s.label)
		expect(labels).toContain('where')
		expect(labels).toContain('set')
		expect(labels).toContain('rename')
		expect(labels).toContain('remove')
		expect(labels).toContain('apply')
		expect(labels).toContain('logs')
		expect(labels).toContain('spans')
		expect(labels).toContain('metrics')
	})
})

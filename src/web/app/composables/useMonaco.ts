// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type * as MonacoType from 'monaco-editor'

let monacoInstance: typeof MonacoType | null = null
let monacoLoadingPromise: Promise<typeof MonacoType> | null = null

export interface MonacoJsonSchemaEntry {
	uri: string
	fileMatch: string[]
	schema: Record<string, any>
}

export function useMonaco() {
	const loadMonaco = async (): Promise<typeof MonacoType> => {
		if (monacoInstance) return monacoInstance

		if (!monacoLoadingPromise) {
			monacoLoadingPromise = (async () => {
				const monaco = await import('monaco-editor/esm/vs/editor/editor.api')
				await import('monaco-editor/esm/vs/language/json/monaco.contribution')
				monacoInstance = monaco as unknown as typeof MonacoType
				return monacoInstance
			})()
		}

		return monacoLoadingPromise
	}

	const registerJsonSchema = (monaco: typeof MonacoType, schemaEntry: MonacoJsonSchemaEntry) => {
		const jsonLanguages = (monaco.languages as any).json
		if (!jsonLanguages?.jsonDefaults) return

		const currentOptions = jsonLanguages.jsonDefaults.diagnosticsOptions || {
			validate: true,
			schemas: []
		}
		const currentSchemas = currentOptions.schemas || []
		const updatedSchemas = currentSchemas.filter((s: any) => s.uri !== schemaEntry.uri)
		updatedSchemas.push(schemaEntry)

		jsonLanguages.jsonDefaults.setDiagnosticsOptions({
			...currentOptions,
			validate: true,
			enableSchemaRequest: true,
			schemas: updatedSchemas
		})
	}

	const unregisterJsonSchema = (monaco: typeof MonacoType, schemaUri: string) => {
		const jsonLanguages = (monaco.languages as any).json
		if (!jsonLanguages?.jsonDefaults) return

		const currentOptions = jsonLanguages.jsonDefaults.diagnosticsOptions
		if (!currentOptions?.schemas) return

		jsonLanguages.jsonDefaults.setDiagnosticsOptions({
			...currentOptions,
			schemas: currentOptions.schemas.filter((s: any) => s.uri !== schemaUri)
		})
	}

	return {
		loadMonaco,
		registerJsonSchema,
		unregisterJsonSchema
	}
}

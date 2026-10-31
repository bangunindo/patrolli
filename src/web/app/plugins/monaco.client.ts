// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import EditorWorker from 'monaco-editor/esm/vs/editor/editor.worker.js?worker'
import JsonWorker from 'monaco-editor/esm/vs/language/json/json.worker.js?worker'

export default defineNuxtPlugin(() => {
	if (typeof window !== 'undefined' && !(window as any).MonacoEnvironment) {
		;(window as any).MonacoEnvironment = {
			getWorker(_: any, label: string) {
				if (label === 'json') {
					return new JsonWorker()
				}
				return new EditorWorker()
			}
		}
	}
})

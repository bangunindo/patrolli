// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

declare module 'monaco-editor/esm/vs/editor/editor.api' {
	export * from 'monaco-editor'
}

declare module 'monaco-editor/esm/vs/language/json/monaco.contribution' {
	// Side-effect import that registers JSON language contribution
}

declare module '*?worker' {
	const workerConstructor: {
		new (): Worker
	}
	export default workerConstructor
}

// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type * as Monaco from 'monaco-editor'

export const OPL_LANGUAGE_ID = 'opl'

export const oplKeywords = [
	'func',
	'extern',
	'let',
	'import',
	'as',
	'is',
	'type',
	'from',
	'define',
	'return',
	'assert',
	'while',
	'for',
	'in',
	'new',
	'this',
	'policy',
	'strategy'
]

export const oplOperators = [
	'where',
	'drop',
	'set',
	'rename',
	'remove',
	'apply',
	'keep',
	'project_keep',
	'if',
	'else',
	'then',
	'route_to',
	'identity',
	'and',
	'or',
	'not'
]

export const oplSignals = [
	'logs',
	'traces',
	'spans',
	'metrics',
	'signals',
	'gauges',
	'sums',
	'histograms',
	'exponential_histograms',
	'summaries'
]

export const oplScopes = ['attributes', 'resource', 'scope', 'record', 'data_points']

export const oplTypeKeywords = [
	'string',
	'int',
	'int32',
	'int64',
	'uint',
	'uint32',
	'uint64',
	'float',
	'float32',
	'float64',
	'number',
	'bool',
	'byte',
	'attributeValue',
	'any',
	'map',
	'timestamp',
	'duration'
]

export const oplBuiltinFunctions = [
	'contains',
	'matches',
	'coalesce',
	'concat',
	'substring',
	'to_lower',
	'to_upper',
	'trim',
	'split',
	'length',
	'replace',
	'hash',
	'md5',
	'sha1',
	'sha256',
	'now',
	'unix_timestamp',
	'is_null',
	'is_empty',
	'is_valid'
]

export const oplConstants = ['true', 'false', 'null']

export const oplLanguageConfiguration: Monaco.languages.LanguageConfiguration = {
	comments: {
		lineComment: '//',
		blockComment: ['/*', '*/']
	},
	brackets: [
		['{', '}'],
		['[', ']'],
		['(', ')']
	],
	autoClosingPairs: [
		{ open: '{', close: '}' },
		{ open: '[', close: ']' },
		{ open: '(', close: ')' },
		{ open: '"', close: '"' },
		{ open: "'", close: "'" }
	],
	surroundingPairs: [
		{ open: '{', close: '}' },
		{ open: '[', close: ']' },
		{ open: '(', close: ')' },
		{ open: '"', close: '"' },
		{ open: "'", close: "'" }
	]
}

export const oplMonarchTokensProvider: Monaco.languages.IMonarchLanguage = {
	defaultToken: '',
	tokenPostfix: '.opl',

	keywords: oplKeywords,
	operators: oplOperators,
	signals: oplSignals,
	scopes: oplScopes,
	typeKeywords: oplTypeKeywords,
	builtinFunctions: oplBuiltinFunctions,
	constants: oplConstants,

	brackets: [
		{ open: '{', close: '}', token: 'delimiter.curly' },
		{ open: '[', close: ']', token: 'delimiter.bracket' },
		{ open: '(', close: ')', token: 'delimiter.parenthesis' }
	],

	tokenizer: {
		root: [
			// Whitespace & Comments
			{ include: '@whitespace' },

			// Pipe operator
			[/\|/, 'delimiter.pipe'],

			// Durations (e.g., 10s, 500ms, 1m, 2h)
			[/\b\d+(\.\d+)?(ns|us|µs|ms|s|m|h|d)\b/, 'number.duration'],

			// Numbers
			[/\b0[xX][0-9a-fA-F]+\b/, 'number.hex'],
			[/\b\d+(\.\d+)?([eE][+-]?\d+)?\b/, 'number'],

			// Strings
			[/"([^"\\]|\\.)*"/, 'string'],
			[/'([^'\\]|\\.)*'/, 'string'],

			// Delimiters
			[/[{}()[\]]/, '@brackets'],
			[/[,;.]/, 'delimiter'],

			// Operators
			[/(=>|==|!=|<=|>=|[<>]|=|\+|-|\*|\/|%)/, 'operator'],

			// Identifiers and Keywords
			[
				/[a-zA-Z_]\w*/,
				{
					cases: {
						'@signals': 'type.signal',
						'@scopes': 'variable.predefined',
						'@operators': 'keyword.operator',
						'@keywords': 'keyword',
						'@typeKeywords': 'type',
						'@constants': 'constant',
						'@builtinFunctions': 'support.function',
						'@default': 'identifier'
					}
				}
			]
		],

		whitespace: [
			[/[ \t\r\n]+/, 'white'],
			[/\/\*/, 'comment', '@comment'],
			[/\/\/.*$/, 'comment']
		],

		comment: [
			[/[^/*]+/, 'comment'],
			[/\*\//, 'comment', '@pop'],
			[/[/*]/, 'comment']
		]
	}
}

let isOplRegistered = false

export function registerOplLanguage(monaco: typeof Monaco): void {
	if (isOplRegistered) return

	const languages = monaco.languages.getLanguages()
	const alreadyExists = languages.some((lang) => lang.id === OPL_LANGUAGE_ID)

	if (!alreadyExists) {
		monaco.languages.register({
			id: OPL_LANGUAGE_ID,
			extensions: ['.opl'],
			aliases: ['OPL', 'opl', 'OpenTelemetry Processing Language'],
			mimetypes: ['text/x-opl']
		})
	}

	monaco.languages.setLanguageConfiguration(OPL_LANGUAGE_ID, oplLanguageConfiguration)
	monaco.languages.setMonarchTokensProvider(OPL_LANGUAGE_ID, oplMonarchTokensProvider)

	// Register snippet completions for common OPL constructs
	monaco.languages.registerCompletionItemProvider(OPL_LANGUAGE_ID, {
		provideCompletionItems: (model, position) => {
			const word = model.getWordUntilPosition(position)
			const range = {
				startLineNumber: position.lineNumber,
				endLineNumber: position.lineNumber,
				startColumn: word.startColumn,
				endColumn: word.endColumn
			}

			const suggestions: Monaco.languages.CompletionItem[] = [
				{
					label: 'where',
					kind: monaco.languages.CompletionItemKind.Keyword,
					insertText: 'where ${1:severity_number >= 17}',
					insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
					detail: 'Filter telemetry records by condition',
					range
				},
				{
					label: 'set',
					kind: monaco.languages.CompletionItemKind.Keyword,
					insertText: 'set attributes["${1:key}"] = "${2:value}"',
					insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
					detail: 'Set or update an attribute value',
					range
				},
				{
					label: 'rename',
					kind: monaco.languages.CompletionItemKind.Keyword,
					insertText: 'rename attributes["${1:old_name}"] as "${2:new_name}"',
					insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
					detail: 'Rename attribute',
					range
				},
				{
					label: 'remove',
					kind: monaco.languages.CompletionItemKind.Keyword,
					insertText: 'remove attributes["${1:key}"]',
					insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
					detail: 'Remove attribute from telemetry record',
					range
				},
				{
					label: 'apply',
					kind: monaco.languages.CompletionItemKind.Keyword,
					insertText: 'apply ${1:data_points} {\n\t$0\n}',
					insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
					detail: 'Apply transformations to nested scope (e.g. data_points)',
					range
				},
				{
					label: 'logs',
					kind: monaco.languages.CompletionItemKind.Snippet,
					insertText:
						'logs\n| where ${1:severity_number >= 17}\n| set attributes["${2:alert}"] = true',
					insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
					detail: 'Starter for logs transformation',
					range
				},
				{
					label: 'spans',
					kind: monaco.languages.CompletionItemKind.Snippet,
					insertText:
						'spans\n| where ${1:attributes["http.status_code"] >= 500}\n| set attributes["${2:error}"] = true',
					insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
					detail: 'Starter for spans transformation',
					range
				},
				{
					label: 'metrics',
					kind: monaco.languages.CompletionItemKind.Snippet,
					insertText:
						'metrics\n| where ${1:name == "http_requests_total"}\n| set attributes["${2:environment}"] = "production"',
					insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
					detail: 'Starter for metrics transformation',
					range
				}
			]

			return { suggestions }
		}
	})

	isOplRegistered = true
}

export interface OplValidationResult {
	isValid: boolean
	error?: string
	line?: number
}

function stripComments(code: string): string {
	let inString: '"' | "'" | null = null
	let inSingleLineComment = false
	let inMultiLineComment = false
	let result = ''
	let i = 0

	while (i < code.length) {
		const char = code[i]
		const nextChar = i + 1 < code.length ? code[i + 1] : ''

		if (inSingleLineComment) {
			if (char === '\n') {
				inSingleLineComment = false
				result += '\n'
			} else {
				result += ' '
			}
			i++
			continue
		}

		if (inMultiLineComment) {
			if (char === '*' && nextChar === '/') {
				inMultiLineComment = false
				result += '  '
				i += 2
			} else {
				result += char === '\n' ? '\n' : ' '
				i++
			}
			continue
		}

		if (inString) {
			result += char
			if (char === '\\' && i + 1 < code.length) {
				result += code[i + 1]
				i += 2
				continue
			}
			if (char === inString) {
				inString = null
			}
			i++
			continue
		}

		if (char === '"' || char === "'") {
			inString = char
			result += char
			i++
			continue
		}

		if (char === '/' && nextChar === '/') {
			inSingleLineComment = true
			result += '  '
			i += 2
			continue
		}

		if (char === '/' && nextChar === '*') {
			inMultiLineComment = true
			result += '  '
			i += 2
			continue
		}

		result += char
		i++
	}

	return result
}

function checkBalancedDelimiters(code: string): { isValid: boolean; error?: string } {
	let inString: '"' | "'" | null = null
	const stack: { char: string; index: number }[] = []
	const pairs: Record<string, string> = { ')': '(', ']': '[', '}': '{' }

	for (let i = 0; i < code.length; i++) {
		const char = code[i]

		if (inString) {
			if (char === '\\') {
				i++
				continue
			}
			if (char === inString) {
				inString = null
			}
			continue
		}

		if (char === '"' || char === "'") {
			inString = char
			continue
		}

		if (char === '(' || char === '[' || char === '{') {
			stack.push({ char, index: i })
		} else if (char === ')' || char === ']' || char === '}') {
			const expected = pairs[char]
			if (stack.length === 0 || stack[stack.length - 1].char !== expected) {
				return {
					isValid: false,
					error: `Tanda kurung '${char}' tidak memiliki pasangan pembuka yang sesuai.`
				}
			}
			stack.pop()
		}
	}

	if (inString) {
		return {
			isValid: false,
			error: 'Tanda kutip string belum tertutup.'
		}
	}

	if (stack.length > 0) {
		const unclosed = stack[stack.length - 1].char
		return {
			isValid: false,
			error: `Tanda kurung '${unclosed}' belum tertutup.`
		}
	}

	return { isValid: true }
}

function splitTopLevelPipes(code: string): string[] {
	let inString: '"' | "'" | null = null
	let parenDepth = 0
	let bracketDepth = 0
	let braceDepth = 0

	const segments: string[] = []
	let currentSegment = ''

	for (let i = 0; i < code.length; i++) {
		const char = code[i]

		if (inString) {
			currentSegment += char
			if (char === '\\' && i + 1 < code.length) {
				currentSegment += code[i + 1]
				i++
				continue
			}
			if (char === inString) {
				inString = null
			}
			continue
		}

		if (char === '"' || char === "'") {
			inString = char
			currentSegment += char
			continue
		}

		if (char === '(') parenDepth++
		else if (char === ')') parenDepth = Math.max(0, parenDepth - 1)
		else if (char === '[') bracketDepth++
		else if (char === ']') bracketDepth = Math.max(0, bracketDepth - 1)
		else if (char === '{') braceDepth++
		else if (char === '}') braceDepth = Math.max(0, braceDepth - 1)

		if (char === '|' && parenDepth === 0 && bracketDepth === 0 && braceDepth === 0) {
			segments.push(currentSegment)
			currentSegment = ''
			continue
		}

		currentSegment += char
	}

	segments.push(currentSegment)
	return segments
}

export function validateOplSyntax(rawCode: string | null | undefined): OplValidationResult {
	if (!rawCode || typeof rawCode !== 'string' || !rawCode.trim()) {
		return {
			isValid: false,
			error: 'Kode OPL tidak boleh kosong.'
		}
	}

	const stripped = stripComments(rawCode)
	if (!stripped.trim()) {
		return {
			isValid: false,
			error: 'Kode OPL tidak boleh kosong.'
		}
	}

	const balanceCheck = checkBalancedDelimiters(stripped)
	if (!balanceCheck.isValid) {
		return {
			isValid: false,
			error: balanceCheck.error
		}
	}

	const rawSegments = splitTopLevelPipes(stripped)
	const segments = rawSegments.map((s) => s.trim())

	if (segments.some((s) => s === '')) {
		return {
			isValid: false,
			error: 'Terdapat segmen pipe (|) kosong atau tidak lengkap.'
		}
	}

	const firstSegment = segments[0]
	const firstTokens = firstSegment.split(/\s+/)
	const signalName = firstTokens[0]

	if (!oplSignals.includes(signalName.toLowerCase())) {
		return {
			isValid: false,
			error: `Program OPL harus diawali dengan target signal (${oplSignals.slice(0, 4).join(', ')}, dll.). Ditemukan '${signalName}'.`
		}
	}

	if (firstTokens.length > 1) {
		return {
			isValid: false,
			error: `Target signal '${signalName}' harus dipisahkan dengan tanda pipe (|) sebelum instruksi '${firstTokens[1]}'.`
		}
	}

	for (let i = 1; i < segments.length; i++) {
		const segment = segments[i]
		const tokens = segment.split(/\s+/)
		const operator = tokens[0].toLowerCase()

		if (!oplOperators.includes(operator)) {
			return {
				isValid: false,
				error: `Operator '${tokens[0]}' pada segmen ke-${i + 1} tidak dikenali. Gunakan salah satu dari: where, set, rename, remove, drop, apply, dll.`
			}
		}

		const restOfSegment = segment.slice(tokens[0].length).trim()

		switch (operator) {
			case 'where':
				if (!restOfSegment) {
					return {
						isValid: false,
						error: "Operator 'where' membutuhkan ekspresi kondisi (contoh: where severity_number >= 17)."
					}
				}
				break

			case 'set':
				if (!restOfSegment) {
					return {
						isValid: false,
						error: 'Operator \'set\' membutuhkan target dan nilai (contoh: set attributes["key"] = "value").'
					}
				}
				if (!restOfSegment.includes('=')) {
					return {
						isValid: false,
						error: 'Operator \'set\' harus memiliki tanda assignment \'=\' (contoh: set attributes["key"] = "value").'
					}
				}
				break

			case 'rename':
				if (!restOfSegment) {
					return {
						isValid: false,
						error: 'Operator \'rename\' membutuhkan target dan nama baru (contoh: rename attributes["old"] as "new").'
					}
				}
				if (!/\bas\b/i.test(restOfSegment)) {
					return {
						isValid: false,
						error: 'Operator \'rename\' harus menggunakan klausa \'as\' (contoh: rename attributes["old"] as "new").'
					}
				}
				break

			case 'remove':
				if (!restOfSegment) {
					return {
						isValid: false,
						error: 'Operator \'remove\' membutuhkan target atribut (contoh: remove attributes["key"]).'
					}
				}
				break

			case 'apply':
				if (!restOfSegment) {
					return {
						isValid: false,
						error: "Operator 'apply' membutuhkan target scope dan blok kurung kurawal '{ ... }' (contoh: apply data_points { ... })."
					}
				}
				if (!restOfSegment.includes('{') || !restOfSegment.includes('}')) {
					return {
						isValid: false,
						error: "Operator 'apply' harus memiliki blok kurung kurawal '{ ... }' (contoh: apply data_points { ... })."
					}
				}
				break
		}
	}

	return { isValid: true }
}

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

// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

/* eslint-env node */
require('@rushstack/eslint-patch/modern-module-resolution')
require('eslint-plugin-unused-imports')

module.exports = {
	root: true,
	ignorePatterns: [
		'**/dist/**',
		'**/node_modules/**',
		'**/playwright/**',
		'**/playwright-report/**',
		'**/test-results/**',
		'**/volumes/**',
		'**/shared/generated/**'
	],
	env: {
		node: true,
		browser: true,
		es2022: true
	},
	globals: {
		definePageMeta: 'readonly',
		defineNuxtPlugin: 'readonly',
		defineNuxtRouteMiddleware: 'readonly',
		navigateTo: 'readonly',
		useRuntimeConfig: 'readonly',
		useNuxtApp: 'readonly',
		useRoute: 'readonly',
		useRouter: 'readonly',
		useFetch: 'readonly',
		useAsyncData: 'readonly',
		useCookie: 'readonly',
		useHead: 'readonly',
		useSeoMeta: 'readonly',
		$fetch: 'readonly'
	},
	plugins: ['unused-imports'],
	extends: [
		'plugin:vue/vue3-essential',
		'eslint:recommended',
		'@vue/eslint-config-typescript',
		'@vue/eslint-config-prettier/skip-formatting'
	],
	parserOptions: { ecmaVersion: 'latest' },
	rules: {
		'no-unused-vars': 'off',
		'no-empty-pattern': 'off',
		'@typescript-eslint/no-unused-vars': 'off',
		'no-undef': 'warn',
		'vue/multi-word-component-names': 'off',
		'vue/valid-v-slot': 'off',
		'vue/no-unused-vars': 'off',
		'vue/no-use-v-if-with-v-for': 'off',
		'vue/require-v-for-key': 'off',
		'unused-imports/no-unused-imports': 'error',
		'unused-imports/no-unused-vars': [
			'warn',
			{
				vars: 'all',
				varsIgnorePattern: '^_|^props$',
				args: 'after-used',
				argsIgnorePattern: '^_'
			}
		]
	}
}

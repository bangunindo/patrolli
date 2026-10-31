// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { fileURLToPath, URL } from 'node:url'

import { defineConfig } from 'vitest/config'

import vue from '@vitejs/plugin-vue'

export default defineConfig({
	plugins: [vue()],

	resolve: {
		alias: {
			'@': fileURLToPath(new URL('./app', import.meta.url)),
			'@@': fileURLToPath(new URL('./', import.meta.url)),
			'~': fileURLToPath(new URL('./app', import.meta.url)),
			'~~': fileURLToPath(new URL('./', import.meta.url))
		}
	},

	test: {
		include: [
			// 'src/tests/unit/**/*.(test|spec).ts', // Unit tests
			// 'src/tests/integration/**/*.(test|spec).ts', // Integration tests
			'tests/**/*.{test,spec}.ts',
			'src/tests/**/*.{test,spec}.ts'
		],
		globals: true,
		environment: 'happy-dom',
		testTimeout: 10000
	}
})

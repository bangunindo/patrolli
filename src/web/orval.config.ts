// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { defineConfig } from 'orval'

export default defineConfig({
	devschema: {
		input: ['http://localhost:8080/api-docs/openapi.json'],
		output: {
			target: './shared/generated',
			client: 'zod',
			mode: 'tags',
			fileExtension: '.type.ts',
			override: {
				zod: {
					generateReusableSchemas: true
				}
			}
		},
		hooks: {
			afterAllFilesWrite: 'prettier --write'
		}
	},
	apischema: {
		input: ['./openapi.json'],
		output: {
			target: './shared/generated',
			client: 'zod',
			mode: 'tags',
			fileExtension: '.type.ts',
			override: {
				zod: {
					generateReusableSchemas: true
				}
			}
		},
		hooks: {
			afterAllFilesWrite: 'prettier --write'
		}
	},
	patrolliapischema: {
		input: ['../core/openapi.json'],
		output: {
			target: './shared/generated',
			client: 'zod',
			mode: 'tags',
			fileExtension: '.type.ts',
			override: {
				zod: {
					generateReusableSchemas: true
				}
			}
		},
		hooks: {
			afterAllFilesWrite: 'prettier --write'
		}
	}
})

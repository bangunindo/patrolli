// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from 'vitest'
import {
	AddAttributes,
	CreateProcessorRequest,
	DropAttributes,
	RenameAttributes
} from '@@/shared/generated/patrolliAPI.schemas.type'
import { introspectZodSchema } from '@/app/services/forms/zod-introspector'
import { processorHints } from '@/app/services/forms/ui-hints/processor.hints'
import { AutoFormAdapter } from '@/app/services/forms/auto-adapter'

describe('Processor Record Fields Dynamic Form Integration', () => {
	describe('DropAttributes (Record of String Array)', () => {
		it('mengintrospeksi DropAttributes dengan tepat', () => {
			const descriptors = introspectZodSchema(DropAttributes, [], processorHints)
			const keysDesc = descriptors.find((d) => d.name === 'keys')!

			expect(keysDesc).toBeDefined()
			expect(keysDesc.widget).toBe('record')
			expect(keysDesc.options).toEqual([
				{ label: 'RECORD', value: 'record' },
				{ label: 'SCOPE', value: 'scope' },
				{ label: 'RESOURCE', value: 'resource' }
			])

			const childDesc = keysDesc.children?.[0]
			expect(childDesc).toBeDefined()
			expect(childDesc?.widget).toBe('array-string')
		})

		it('AutoFormAdapter membersihkan dan memvalidasi draft DropAttributes', () => {
			const draft = {
				keys: {
					record: ['http.status_code', 'http.method', ''],
					scope: ['otel.library.name'],
					'  ': ['should_be_stripped']
				}
			}

			const result = AutoFormAdapter.sanitizeAndValidate(
				DropAttributes,
				draft,
				processorHints
			)
			expect(result.isValid).toBe(true)
			expect(result.data).toEqual({
				keys: {
					record: ['http.status_code', 'http.method'],
					scope: ['otel.library.name']
				}
			})
		})
	})

	describe('RenameAttributes (Nested Record of Strings)', () => {
		it('mengintrospeksi RenameAttributes secara rekursif', () => {
			const descriptors = introspectZodSchema(RenameAttributes, [], processorHints)
			const renameDesc = descriptors.find((d) => d.name === 'rename_map')!

			expect(renameDesc).toBeDefined()
			expect(renameDesc.widget).toBe('record')
			expect(renameDesc.options).toEqual([
				{ label: 'RECORD', value: 'record' },
				{ label: 'SCOPE', value: 'scope' },
				{ label: 'RESOURCE', value: 'resource' }
			])

			// First child is inner record (table -> (old -> new))
			const innerRecordDesc = renameDesc.children?.[0]
			expect(innerRecordDesc).toBeDefined()
			expect(innerRecordDesc?.widget).toBe('record')

			// Child of inner record is string (new name)
			const innerValueDesc = innerRecordDesc?.children?.[0]
			expect(innerValueDesc).toBeDefined()
			expect(innerValueDesc?.widget).toBe('text')
		})

		it('AutoFormAdapter membersihkan dan memvalidasi draft RenameAttributes', () => {
			const draft = {
				rename_map: {
					record: {
						old_key: 'new_key',
						'   ': 'empty_key_stripped'
					},
					scope: {
						service_name: 'service.name'
					},
					'': {
						foo: 'bar'
					}
				}
			}

			const result = AutoFormAdapter.sanitizeAndValidate(
				RenameAttributes,
				draft,
				processorHints
			)
			expect(result.isValid).toBe(true)
			expect(result.data).toEqual({
				rename_map: {
					record: {
						old_key: 'new_key'
					},
					scope: {
						service_name: 'service.name'
					}
				}
			})
		})
	})

	describe('AddAttributes (Record of Record of Discriminated Union)', () => {
		it('mengintrospeksi AddAttributes secara rekursif ke discriminated union', () => {
			const descriptors = introspectZodSchema(AddAttributes, [], processorHints)
			const kvMapDesc = descriptors.find((d) => d.name === 'kv_map')!
			const overwriteDesc = descriptors.find((d) => d.name === 'overwrite')!

			expect(overwriteDesc).toBeDefined()
			expect(overwriteDesc.widget).toBe('switch')

			expect(kvMapDesc).toBeDefined()
			expect(kvMapDesc.widget).toBe('record')
			expect(kvMapDesc.options).toEqual([
				{ label: 'RECORD', value: 'record' },
				{ label: 'SCOPE', value: 'scope' },
				{ label: 'RESOURCE', value: 'resource' }
			])

			const innerRecordDesc = kvMapDesc.children?.[0]
			expect(innerRecordDesc).toBeDefined()
			expect(innerRecordDesc?.widget).toBe('record')

			// TransformValue discriminated union
			const unionDesc = innerRecordDesc?.children?.[0]
			expect(unionDesc).toBeDefined()
			expect(unionDesc?.widget).toBe('discriminated-union')
			expect(unionDesc?.unionDiscriminator).toBe('type')
			expect(unionDesc?.unionBranches).toBeDefined()
			expect(Object.keys(unionDesc?.unionBranches || {})).toEqual([
				'bool',
				'double',
				'int',
				'str'
			])
		})

		it('AutoFormAdapter membersihkan dan meng-coerce tipe data TransformValue', () => {
			const draft = {
				overwrite: true,
				kv_map: {
					record: {
						'http.status': { type: 'int', value: '200' },
						'is.debug': { type: 'bool', value: false },
						'host.weight': { type: 'double', value: '1.75' },
						'env.name': { type: 'str', value: 'production' }
					}
				}
			}

			const result = AutoFormAdapter.sanitizeAndValidate(AddAttributes, draft, processorHints)
			expect(result.isValid).toBe(true)
			expect(result.data).toEqual({
				overwrite: true,
				kv_map: {
					record: {
						'http.status': { type: 'int', value: 200 },
						'is.debug': { type: 'bool', value: false },
						'host.weight': { type: 'double', value: 1.75 },
						'env.name': { type: 'str', value: 'production' }
					}
				}
			})
		})
	})

	describe('CreateProcessorRequest Integration', () => {
		it('berhasil memvalidasi processor dengan modul drop_attributes', () => {
			const payload = {
				adm_status: 1,
				name: 'Drop Attributes Processor',
				input_items: [],
				input_processors: [],
				config: {
					type: 'drop_attributes',
					config: {
						keys: {
							record: ['user.password', 'secret.token']
						}
					}
				}
			}

			const result = AutoFormAdapter.sanitizeAndValidate(
				CreateProcessorRequest,
				payload,
				processorHints
			)
			expect(result.isValid).toBe(true)
			const config = result.data?.config as any
			expect(config.type ?? config.module).toBe('drop_attributes')
			expect(config.config.keys.record).toEqual(['user.password', 'secret.token'])
		})

		it('berhasil memvalidasi processor dengan modul rename_attributes', () => {
			const payload = {
				adm_status: 1,
				name: 'Rename Attributes Processor',
				input_items: [],
				input_processors: [],
				config: {
					type: 'rename_attributes',
					config: {
						rename_map: {
							record: {
								'http.url': 'url.full'
							}
						}
					}
				}
			}

			const result = AutoFormAdapter.sanitizeAndValidate(
				CreateProcessorRequest,
				payload,
				processorHints
			)
			expect(result.isValid).toBe(true)
			const config = result.data?.config as any
			expect(config.type ?? config.module).toBe('rename_attributes')
			expect(config.config.rename_map.record['http.url']).toBe('url.full')
		})

		it('berhasil memvalidasi processor dengan modul add_attributes', () => {
			const payload = {
				adm_status: 1,
				name: 'Add Attributes Processor',
				input_items: [],
				input_processors: [],
				config: {
					type: 'add_attributes',
					config: {
						overwrite: false,
						kv_map: {
							resource: {
								cluster: { type: 'str', value: 'k8s-prod-1' }
							}
						}
					}
				}
			}

			const result = AutoFormAdapter.sanitizeAndValidate(
				CreateProcessorRequest,
				payload,
				processorHints
			)
			expect(result.isValid).toBe(true)
			const config = result.data?.config as any
			expect(config.type ?? config.module).toBe('add_attributes')
			expect(config.config.kv_map.resource.cluster).toEqual({
				type: 'str',
				value: 'k8s-prod-1'
			})
		})
	})
})

// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from 'vitest'
import { reactive } from 'vue'
import {
	deleteValueByPath,
	formatFieldLabel,
	getValueByPath,
	isDeepEqual,
	pathToString,
	resolveHintForPath,
	setValueByPath,
	stringToPath
} from '@/app/services/forms/path-helper'

describe('path-helper', () => {
	describe('getValueByPath & setValueByPath', () => {
		it('should get nested value correctly', () => {
			const obj = {
				config: {
					snmp_v3: {
						username: 'admin'
					}
				}
			}

			expect(getValueByPath(obj, ['config', 'snmp_v3', 'username'])).toBe('admin')
			expect(getValueByPath(obj, ['config', 'snmp_v3', 'password'])).toBeUndefined()
			expect(getValueByPath(obj, ['non_existent'])).toBeUndefined()
		})

		it('should set nested value and create intermediate objects if they do not exist', () => {
			const obj: Record<string, any> = {}
			setValueByPath(obj, ['config', 'http', 'url'], 'http://localhost:8080')

			expect(obj.config).toBeDefined()
			expect(obj.config.http).toBeDefined()
			expect(obj.config.http.url).toBe('http://localhost:8080')
		})

		it('should delete nested value correctly', () => {
			const obj = {
				config: {
					old_branch: { value: 123 },
					keep_branch: { value: 456 }
				}
			}

			deleteValueByPath(obj, ['config', 'old_branch'])
			expect(obj.config.old_branch).toBeUndefined()
			expect(obj.config.keep_branch).toBeDefined()
		})
	})

	describe('pathToString & stringToPath', () => {
		it('should convert array to string and vice versa', () => {
			const path = ['config', 'snmp', 'address']
			const str = pathToString(path)
			expect(str).toBe('config.snmp.address')
			expect(stringToPath(str)).toEqual(path)
		})
	})

	describe('formatFieldLabel', () => {
		it('should convert snake_case to Title Case with proper acronyms', () => {
			expect(formatFieldLabel('name')).toBe('Name')
			expect(formatFieldLabel('host_id')).toBe('Host ID')
			expect(formatFieldLabel('max_bulk_repetitions')).toBe('Max Bulk Repetitions')
			expect(formatFieldLabel('connect_timeout_ms')).toBe('Connect Timeout MS')
			expect(formatFieldLabel('snmp_community')).toBe('SNMP Community')
			expect(formatFieldLabel('tls_cert_path')).toBe('TLS Cert Path')
		})

		it('should convert camelCase to Title Case', () => {
			expect(formatFieldLabel('workerCount')).toBe('Worker Count')
			expect(formatFieldLabel('disableClusterForwarding')).toBe('Disable Cluster Forwarding')
		})
	})

	describe('resolveHintForPath', () => {
		it('should prioritize branch-specific hints over generic config.config hints for direct and array paths', () => {
			const hints = {
				'config.config': { hideGroup: true },
				'config.attribute_filter.config': { widget: 'attribute-filter', hideGroup: true },
				'config.config.batch_size': { label: 'Generic Batch Size' },
				'config.open_telemetry.batch_size': { label: 'OTel Batch Size' },
				'config.config.items.name': { label: 'Generic Item Name' },
				'config.internal_logs.items.name': { label: 'Module Name' }
			}

			expect(
				resolveHintForPath('config.config', hints, { config: { type: 'attribute_filter' } })
					?.widget
			).toBe('attribute-filter')

			expect(
				resolveHintForPath('config.config.batch_size', hints, {
					config: { type: 'open_telemetry' }
				})?.label
			).toBe('OTel Batch Size')

			expect(
				resolveHintForPath('config.config.batch_size', hints, {
					config: { type: 'other_type' }
				})?.label
			).toBe('Generic Batch Size')

			expect(
				resolveHintForPath('config.config.items.0.name', hints, {
					config: { type: 'internal_logs' }
				})?.label
			).toBe('Module Name')

			expect(
				resolveHintForPath('config.config.items.0.name', hints, {
					config: { type: 'other_type' }
				})?.label
			).toBe('Generic Item Name')
		})

		it('should resolve 2-segment .config hints symmetrically for non-config root union sections', () => {
			const hints = {
				'channel.config': { label: 'Generic Channel Config' },
				'channel.disk.config': { label: 'Disk Channel Config', hideGroup: true }
			}

			expect(
				resolveHintForPath('channel.config', hints, { channel: { type: 'disk' } })
			).toEqual({
				label: 'Disk Channel Config',
				hideGroup: true
			})

			expect(
				resolveHintForPath('channel.config', hints, { channel: { type: 'memory' } })?.label
			).toBe('Generic Channel Config')
		})
	})

	describe('isDeepEqual', () => {
		it('should compare primitives, NaN, null, undefined, and Date values accurately', () => {
			expect(isDeepEqual(1, 1)).toBe(true)
			expect(isDeepEqual('abc', 'abc')).toBe(true)
			expect(isDeepEqual(NaN, NaN)).toBe(true)
			expect(isDeepEqual(null, null)).toBe(true)
			expect(isDeepEqual(undefined, undefined)).toBe(true)
			expect(isDeepEqual(null, undefined)).toBe(false)
			expect(isDeepEqual(1, '1')).toBe(false)
			expect(isDeepEqual(new Date('2026-01-01'), new Date('2026-01-01'))).toBe(true)
			expect(isDeepEqual(new Date('2026-01-01'), new Date('2026-01-02'))).toBe(false)
			expect(isDeepEqual(new Date('2026-01-01'), {})).toBe(false)
		})

		it('should compare nested objects regardless of key order and unwrap Vue reactive proxies', () => {
			const a = reactive({ name: 'test', config: { b: 2, a: 1 }, tags: ['x', 'y'] })
			const b = { tags: ['x', 'y'], config: { a: 1, b: 2 }, name: 'test' }

			expect(isDeepEqual(a, b)).toBe(true)
			expect(isDeepEqual(a, { ...b, tags: ['y', 'x'] })).toBe(false)
			expect(isDeepEqual(a, { ...b, extra: undefined })).toBe(false)
			expect(isDeepEqual([1, 2], { 0: 1, 1: 2 })).toBe(false)
		})

		it('should handle circular references in objects and arrays without stack overflow', () => {
			const objA: Record<string, any> = { name: 'node' }
			objA.self = objA
			const objB: Record<string, any> = { name: 'node' }
			objB.self = objB
			const objC: Record<string, any> = { name: 'other' }
			objC.self = objC

			expect(isDeepEqual(objA, objB)).toBe(true)
			expect(isDeepEqual(objA, objC)).toBe(false)

			const arrA: any[] = [1]
			arrA.push(arrA)
			const arrB: any[] = [1]
			arrB.push(arrB)
			const arrC: any[] = [2]
			arrC.push(arrC)

			expect(isDeepEqual(arrA, arrB)).toBe(true)
			expect(isDeepEqual(arrA, arrC)).toBe(false)
		})
	})
})

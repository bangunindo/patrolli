// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from 'vitest'
import {
	deleteValueByPath,
	formatFieldLabel,
	getValueByPath,
	pathToString,
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
})

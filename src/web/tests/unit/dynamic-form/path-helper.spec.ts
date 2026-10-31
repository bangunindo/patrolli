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
		it('mengambil nilai bersarang dengan benar', () => {
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

		it('menetapkan nilai bersarang dan membuat objek perantara jika belum ada', () => {
			const obj: Record<string, any> = {}
			setValueByPath(obj, ['config', 'http', 'url'], 'http://localhost:8080')

			expect(obj.config).toBeDefined()
			expect(obj.config.http).toBeDefined()
			expect(obj.config.http.url).toBe('http://localhost:8080')
		})

		it('menghapus nilai bersarang dengan benar', () => {
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
		it('mengonversi array ke string dan sebaliknya', () => {
			const path = ['config', 'snmp', 'address']
			const str = pathToString(path)
			expect(str).toBe('config.snmp.address')
			expect(stringToPath(str)).toEqual(path)
		})
	})

	describe('formatFieldLabel', () => {
		it('mengonversi snake_case ke Title Case dengan akronim yang tepat', () => {
			expect(formatFieldLabel('name')).toBe('Name')
			expect(formatFieldLabel('host_id')).toBe('Host ID')
			expect(formatFieldLabel('max_bulk_repetitions')).toBe('Max Bulk Repetitions')
			expect(formatFieldLabel('connect_timeout_ms')).toBe('Connect Timeout MS')
			expect(formatFieldLabel('snmp_community')).toBe('SNMP Community')
			expect(formatFieldLabel('tls_cert_path')).toBe('TLS Cert Path')
		})

		it('mengonversi camelCase ke Title Case', () => {
			expect(formatFieldLabel('workerCount')).toBe('Worker Count')
			expect(formatFieldLabel('disableClusterForwarding')).toBe('Disable Cluster Forwarding')
		})
	})
})

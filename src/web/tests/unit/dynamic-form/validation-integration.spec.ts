// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from 'vitest'
import { z } from 'zod'
import { introspectZodSchema } from '@/app/services/forms/zod-introspector'
import { AutoFormAdapter } from '@/app/services/forms/auto-adapter'
import {
	registerFormatValidator,
	validateFormat,
	hasFormatValidator,
	getFormatValidator
} from '@/app/services/forms/format-registry'

describe('Dynamic Form Validation & Format Registry Integration', () => {
	describe('FormatValidatorRegistry', () => {
		const ctx = { path: ['test'], pathKey: 'test', label: 'Test Field' }

		it('memvalidasi format UUID bawaan', () => {
			expect(validateFormat('uuid', '123e4567-e89b-12d3-a456-426614174000', ctx)).toBeNull()
			expect(validateFormat('uuid', 'invalid-uuid', ctx)).toBe(
				'Test Field must be a valid UUID'
			)
		})

		it('memvalidasi format Email bawaan', () => {
			expect(validateFormat('email', 'admin@patrolli.io', ctx)).toBeNull()
			expect(validateFormat('email', 'not-an-email', ctx)).toBe(
				'Test Field must be a valid email address'
			)
		})

		it('memvalidasi format IPv4, IPv6, dan CIDR', () => {
			// IPv4
			expect(validateFormat('ipv4', '192.168.1.1', ctx)).toBeNull()
			expect(validateFormat('ipv4', '999.999.999.999', ctx)).toBe(
				'Test Field must be a valid IPv4 address (e.g. 192.168.1.1)'
			)

			// IPv6
			expect(
				validateFormat('ipv6', '2001:0db8:85a3:0000:0000:8a2e:0370:7334', ctx)
			).toBeNull()
			expect(validateFormat('ipv6', '127.0.0.1', ctx)).toBe(
				'Test Field must be a valid IPv6 address'
			)

			// Generic IP (v4 or v6)
			expect(validateFormat('ip', '10.0.0.1', ctx)).toBeNull()
			expect(validateFormat('ip', '::1', ctx)).toBeNull()
			expect(validateFormat('ip', 'invalid-ip', ctx)).toBe(
				'Test Field must be a valid IP address'
			)

			// IP or CIDR
			expect(validateFormat('ip-or-cidr', '10.0.0.0/24', ctx)).toBeNull()
			expect(validateFormat('ip-or-cidr', '172.16.0.1', ctx)).toBeNull()
			expect(validateFormat('ip-or-cidr', '10.0.0.0/99', ctx)).toBe(
				'Test Field must be a valid IP address or CIDR subnet (e.g. 10.0.0.0/24)'
			)
		})

		it('memvalidasi format Port (1-65535)', () => {
			expect(validateFormat('port', 80, ctx)).toBeNull()
			expect(validateFormat('port', '8080', ctx)).toBeNull()
			expect(validateFormat('port', 0, ctx)).toBe(
				'Test Field must be a valid port number (1-65535)'
			)
			expect(validateFormat('port', 70000, ctx)).toBe(
				'Test Field must be a valid port number (1-65535)'
			)
		})

		it('memvalidasi format Hostname dan URL', () => {
			expect(validateFormat('hostname', 'api.patrolli.local', ctx)).toBeNull()
			expect(validateFormat('hostname', '-invalid-.domain', ctx)).toBe(
				'Test Field must be a valid hostname or domain'
			)

			expect(validateFormat('url', 'https://localhost:3000/api', ctx)).toBeNull()
			expect(validateFormat('url', 'ftp://invalid-proto', ctx)).toBe(
				'Test Field must be a valid URL (http, https, ws, wss)'
			)
		})

		it('mendukung registrasi custom format validator baru secara modular', () => {
			const MAC_REGEX = /^([0-9A-Fa-f]{2}[:-]){5}([0-9A-Fa-f]{2})$/
			registerFormatValidator({
				name: 'mac-address',
				test: (val) => {
					if (!val || typeof val !== 'string') return true
					return MAC_REGEX.test(val.trim())
				},
				message: (c) => `${c.label} must be a valid MAC address (e.g. 00:1A:2B:3C:4D:5E)`
			})

			expect(hasFormatValidator('mac-address')).toBe(true)
			expect(getFormatValidator('mac-address')).toBeDefined()

			expect(validateFormat('mac-address', '00:1A:2B:3C:4D:5E', ctx)).toBeNull()
			expect(validateFormat('mac-address', 'invalid-mac', ctx)).toBe(
				'Test Field must be a valid MAC address (e.g. 00:1A:2B:3C:4D:5E)'
			)
		})
	})

	describe('Zod Schema Constraint Introspection', () => {
		it('mengintrospeksi checks pada ZodString otomatis', () => {
			const TestSchema = z.object({
				id: z.string().uuid(),
				email: z.string().email(),
				ip_addr: z.string().ip(),
				website: z.string().url(),
				code: z.string().regex(/^[A-Z]{3}$/),
				bio: z.string().min(10).max(100)
			})

			const descriptors = introspectZodSchema(TestSchema)

			const idDesc = descriptors.find((d) => d.name === 'id')!
			expect(idDesc.format).toBe('uuid')
			expect(idDesc.rules).toContain('uuid')

			const emailDesc = descriptors.find((d) => d.name === 'email')!
			expect(emailDesc.format).toBe('email')
			expect(emailDesc.rules).toContain('email')

			const ipDesc = descriptors.find((d) => d.name === 'ip_addr')!
			expect(ipDesc.format).toBe('ip')
			expect(ipDesc.rules).toContain('ip')

			const urlDesc = descriptors.find((d) => d.name === 'website')!
			expect(urlDesc.format).toBe('url')
			expect(urlDesc.rules).toContain('url')

			const codeDesc = descriptors.find((d) => d.name === 'code')!
			expect(codeDesc.pattern).toBe('^[A-Z]{3}$')
			expect(codeDesc.regex).toBeDefined()

			const bioDesc = descriptors.find((d) => d.name === 'bio')!
			expect(bioDesc.min).toBe(10)
			expect(bioDesc.max).toBe(100)
		})

		it('mengintrospeksi checks pada ZodNumber otomatis', () => {
			const TestSchema = z.object({
				port: z.number().int().min(1).max(65535)
			})

			const descriptors = introspectZodSchema(TestSchema)
			const portDesc = descriptors.find((d) => d.name === 'port')!

			expect(portDesc.min).toBe(1)
			expect(portDesc.max).toBe(65535)
			expect(portDesc.step).toBe(1)
		})

		it('memungkinkan override format dan rules melalui UI hints', () => {
			const TestSchema = z.object({
				custom_ip: z.string()
			})

			const descriptors = introspectZodSchema(TestSchema, [], {
				custom_ip: {
					format: 'ip-or-cidr',
					rules: ['required', 'ip-or-cidr']
				}
			})

			const customIpDesc = descriptors.find((d) => d.name === 'custom_ip')!
			expect(customIpDesc.format).toBe('ip-or-cidr')
			expect(customIpDesc.rules).toEqual(['required', 'ip-or-cidr'])
		})
	})

	describe('AutoFormAdapter Validation & Humanized Error Messages', () => {
		const UserSchema = z.object({
			user_id: z.string().uuid(),
			email: z.string().email(),
			age: z.number().int().min(18),
			username: z.string().min(3)
		})

		it('menghasilkan pesan error yang ramah pengguna saat data tidak valid', () => {
			const invalidDraft = {
				user_id: 'bad-id',
				email: 'bad-email',
				age: 15,
				username: 'ab'
			}

			const res = AutoFormAdapter.toRequest(UserSchema, invalidDraft)
			expect(res.success).toBe(false)
			if (!res.success) {
				expect(res.issues['user_id']).toBe('User ID must be a valid UUID')
				expect(res.issues['email']).toBe('Email must be a valid email address')
				expect(res.issues['age']).toBe('Age must be greater than or equal to 18')
				expect(res.issues['username']).toBe('Username must be at least 3 characters')
			}
		})

		it('menangani field required saat dikosongkan', () => {
			const res = AutoFormAdapter.toRequest(UserSchema, {})
			expect(res.success).toBe(false)
			if (!res.success) {
				expect(res.issues['user_id']).toBe('User ID is required')
				expect(res.issues['email']).toBe('Email is required')
				expect(res.issues['age']).toBe('Age is required')
				expect(res.issues['username']).toBe('Username is required')
			}
		})

		it('mengeksekusi custom validator function dari UI hints', () => {
			const SimpleSchema = z.object({
				code: z.string()
			})

			const hints = {
				code: {
					validator: (val: any) => {
						if (val !== 'SECRET_KEY') {
							return 'Code harus bernilai SECRET_KEY'
						}
						return true
					}
				}
			}

			const failRes = AutoFormAdapter.toRequest(SimpleSchema, { code: 'WRONG' }, hints)
			expect(failRes.success).toBe(false)
			if (!failRes.success) {
				expect(failRes.issues['code']).toBe('Code harus bernilai SECRET_KEY')
			}

			const passRes = AutoFormAdapter.toRequest(SimpleSchema, { code: 'SECRET_KEY' }, hints)
			expect(passRes.success).toBe(true)
		})

		it('menjalankan method validateField untuk validasi per-field secara terpisah', () => {
			const draft = { user_id: 'not-a-uuid', email: 'valid@patrolli.io' }
			const error = AutoFormAdapter.validateField(UserSchema, 'user_id', draft)
			expect(error).toBe('User ID must be a valid UUID')

			const validEmailError = AutoFormAdapter.validateField(UserSchema, 'email', draft)
			expect(validEmailError).toBeNull()
		})
	})

	describe('Nested Form Validation Support', () => {
		const NestedChildSchema = z.object({
			address: z.string().min(1),
			port: z.number().int().min(1).max(65535)
		})

		const NestedParentSchema = z.object({
			name: z.string().min(1),
			listener: NestedChildSchema
		})

		const NestedUnionSchema = z.object({
			name: z.string(),
			config: z.union([
				z.object({
					type: z.literal('udp'),
					config: NestedChildSchema
				}),
				z.object({
					type: z.literal('tcp'),
					config: z.object({ host: z.string() })
				})
			])
		})

		it('memvalidasi nested object dengan validateField menggunakan path relatif maupun absolut', () => {
			const draft = {
				name: 'Test',
				listener: {
					address: '',
					port: 70000
				}
			}

			// Validation via absolute path
			const addrErr = AutoFormAdapter.validateField(
				NestedParentSchema,
				'listener.address',
				draft
			)
			expect(addrErr).toBeDefined()
			expect(addrErr).toMatch(/required|empty/)

			// Validation via leaf field name (suffix)
			const portErr = AutoFormAdapter.validateField(NestedParentSchema, 'port', draft)
			expect(portErr).toBeDefined()
			expect(portErr).toContain('less than or equal to 65535')
		})

		it('memvalidasi sub-scope nested form dengan validateScope', () => {
			const draft = {
				name: 'Test',
				listener: {
					address: '',
					port: 0
				}
			}

			const listenerScopeErrors = AutoFormAdapter.validateScope(
				NestedParentSchema,
				'listener',
				draft
			)
			expect(Object.keys(listenerScopeErrors).length).toBe(2)
			expect(listenerScopeErrors['listener.address']).toBeDefined()
			expect(listenerScopeErrors['listener.port']).toBeDefined()
		})

		it('memvalidasi nested union branch dengan validateField dan validateScope', () => {
			const draft = {
				name: 'My Service',
				config: {
					type: 'udp',
					config: {
						address: '',
						port: 99999
					}
				}
			}

			// validateField with branch alias
			const aliasErr = AutoFormAdapter.validateField(
				NestedUnionSchema,
				'config.udp.config.address',
				draft
			)
			expect(aliasErr).toBeDefined()

			// validateField with leaf name
			const leafPortErr = AutoFormAdapter.validateField(NestedUnionSchema, 'port', draft)
			expect(leafPortErr).toBeDefined()

			// validateScope on 'config' sub-branch
			const configErrors = AutoFormAdapter.validateScope(NestedUnionSchema, 'config', draft)
			expect(Object.keys(configErrors).length).toBeGreaterThanOrEqual(2)
		})
	})

	describe('useValidation Unified Format Registry Integration', () => {
		it('menguji bahwa useValidation menggunakan format validator yang terdaftar', async () => {
			const { useValidation } = await import('@/composables/useValidation')
			const val = '192.168.1.1'
			const rules = ['required', 'ipv4']
			const { messages } = useValidation('Host IP', val, rules)

			// Since isOnSubmit is false before submit, initial messages are empty
			expect(messages.value).toEqual([])
		})

		it('memungkinkan menutup alert dengan mengubah showMessages menjadi false (close alert)', async () => {
			const { useValidation } = await import('@/composables/useValidation')
			const { ref } = await import('vue')
			const val = ref('')
			const rules = ['required']
			const { showMessages, messages } = useValidation('Test Field', val, rules)

			// Set error message
			messages.value = ['Test Field is required']
			expect(showMessages.value).toBe(true)

			// Simulate clicking close button on b-alert
			showMessages.value = false
			expect(showMessages.value).toBe(false)
			expect(messages.value).toEqual([])
		})
	})
})

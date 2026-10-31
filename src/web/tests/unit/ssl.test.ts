// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import fs from 'node:fs'
import path from 'node:path'
import { normalizeWebSocketUrl } from '@/app/services/websocket.service'
import { getSslConfig, isRequestSecure, loadSslConfig } from '@@/server/services/ssl.service'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

describe('SSL Configuration & Security Utilities', () => {
	const originalEnv = process.env

	beforeEach(() => {
		process.env = { ...originalEnv }
		delete process.env.COOKIE_SECURE
		delete process.env.CONFIG_FILE
		delete process.env.CONFIG_PATH
		delete process.env.FRONTEND_CONFIG
	})

	afterEach(() => {
		process.env = originalEnv
		vi.restoreAllMocks()
	})

	describe('getSslConfig & loadSslConfig', () => {
		it('should return null when config file does not have ssl block', () => {
			const dummyConfig = path.resolve(process.cwd(), 'tests/scratch-test-config.yaml')
			fs.writeFileSync(dummyConfig, 'clusters:\n  - name: test\n    url: http://test:8080\n')

			try {
				const ssl = getSslConfig(dummyConfig)
				expect(ssl).toBeNull()

				const loaded = loadSslConfig(dummyConfig)
				expect(loaded).toBeNull()
			} finally {
				if (fs.existsSync(dummyConfig)) fs.unlinkSync(dummyConfig)
			}
		})

		it('should return null when ssl is explicitly disabled (enabled: false)', () => {
			const dummyConfig = path.resolve(process.cwd(), 'tests/scratch-test-disabled-ssl.yaml')
			fs.writeFileSync(
				dummyConfig,
				'ssl:\n  enabled: false\n  cert: ./cert.pem\n  key: ./key.pem\n'
			)

			try {
				const ssl = getSslConfig(dummyConfig)
				expect(ssl?.enabled).toBe(false)

				const loaded = loadSslConfig(dummyConfig)
				expect(loaded).toBeNull()
			} finally {
				if (fs.existsSync(dummyConfig)) fs.unlinkSync(dummyConfig)
			}
		})

		it('should return loaded cert and key when valid files exist', () => {
			const certFile = path.resolve(process.cwd(), 'tests/scratch-cert.pem')
			const keyFile = path.resolve(process.cwd(), 'tests/scratch-key.pem')
			const dummyConfig = path.resolve(process.cwd(), 'tests/scratch-valid-ssl.yaml')

			fs.writeFileSync(
				certFile,
				'-----BEGIN CERTIFICATE-----\nMOCK_CERT\n-----END CERTIFICATE-----'
			)
			fs.writeFileSync(
				keyFile,
				'-----BEGIN PRIVATE KEY-----\nMOCK_KEY\n-----END PRIVATE KEY-----'
			)
			fs.writeFileSync(
				dummyConfig,
				`ssl:\n  enabled: true\n  cert: ${certFile}\n  key: ${keyFile}\n`
			)

			try {
				const loaded = loadSslConfig(dummyConfig)
				expect(loaded).not.toBeNull()
				expect(loaded?.cert).toContain('MOCK_CERT')
				expect(loaded?.key).toContain('MOCK_KEY')
				expect(loaded?.certPath).toBe(certFile)
				expect(loaded?.keyPath).toBe(keyFile)
			} finally {
				if (fs.existsSync(certFile)) fs.unlinkSync(certFile)
				if (fs.existsSync(keyFile)) fs.unlinkSync(keyFile)
				if (fs.existsSync(dummyConfig)) fs.unlinkSync(dummyConfig)
			}
		})
	})

	describe('isRequestSecure', () => {
		it('should return true when COOKIE_SECURE=true is set', () => {
			process.env.COOKIE_SECURE = 'true'
			expect(isRequestSecure()).toBe(true)
		})

		it('should return false when COOKIE_SECURE=false is set', () => {
			process.env.COOKIE_SECURE = 'false'
			expect(isRequestSecure()).toBe(false)
		})

		it('should detect direct encrypted socket', () => {
			const mockEvent = {
				node: {
					req: {
						socket: { encrypted: true },
						headers: {}
					}
				}
			} as any
			expect(isRequestSecure(mockEvent)).toBe(true)
		})

		it('should detect x-forwarded-proto https header', () => {
			const mockEvent = {
				node: {
					req: {
						socket: { encrypted: false },
						headers: {
							'x-forwarded-proto': 'https'
						}
					}
				}
			} as any
			expect(isRequestSecure(mockEvent)).toBe(true)
		})
	})

	describe('normalizeWebSocketUrl', () => {
		it('should convert relative paths to ws url', () => {
			const result = normalizeWebSocketUrl('/ws/pipelines/123')
			expect(result).toMatch(/^ws:\/\//)
			expect(result).toContain('/ws/pipelines/123')
		})

		it('should convert http:// to ws://', () => {
			const result = normalizeWebSocketUrl('http://patrolli-node:8080/ws/capture')
			expect(result).toBe('ws://patrolli-node:8080/ws/capture')
		})

		it('should convert https:// to wss://', () => {
			const result = normalizeWebSocketUrl('https://patrolli-node:8080/ws/capture')
			expect(result).toBe('wss://patrolli-node:8080/ws/capture')
		})
	})
})

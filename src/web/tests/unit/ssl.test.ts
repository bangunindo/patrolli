// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import fs from 'node:fs'
import path from 'node:path'
import tls from 'node:tls'
import { createPinia, setActivePinia } from 'pinia'
import { AppWebSocket, normalizeWebSocketUrl } from '@/app/services/websocket.service'
import { useAuthStore } from '@/stores/auth.store'
import { clearServerHttpClientCache, getServerHttpClient } from '@@/server/services/apibase.service'
import * as sslService from '@@/server/services/ssl.service'
import {
	clearCaCertCache,
	getSslConfig,
	isRequestSecure,
	loadSslConfig,
	resolveTlsConnectOptions
} from '@@/server/services/ssl.service'
import { setUserClustersCookie } from '@@/server/utils/auth-cookies'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

describe('SSL Configuration & Security Utilities', () => {
	const originalEnv = process.env

	beforeEach(() => {
		process.env = { ...originalEnv }
		delete process.env.COOKIE_SECURE
		delete process.env.CONFIG_FILE
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

		it('should detect multi-hop x-forwarded-proto headers case-insensitively', () => {
			process.env.CONFIG_FILE = '/non-existent-ssl-config.yaml'
			process.env.NODE_ENV = 'development'

			const httpsFirstEvent = {
				node: {
					req: {
						socket: { encrypted: false },
						headers: {
							'x-forwarded-proto': 'HTTPS, http'
						}
					}
				}
			} as any
			expect(isRequestSecure(httpsFirstEvent)).toBe(true)

			const httpFirstEvent = {
				node: {
					req: {
						socket: { encrypted: false },
						headers: {
							'x-forwarded-proto': 'http, https'
						}
					}
				}
			} as any
			expect(isRequestSecure(httpFirstEvent)).toBe(false)
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

	describe('setUserClustersCookie secure flag consistency', () => {
		it('should set Secure flag when isRequestSecure returns true even outside production', () => {
			process.env.NODE_ENV = 'development'
			process.env.COOKIE_SECURE = 'true'

			const headersMap = new Map<string, any>()
			const mockEvent = {
				node: {
					req: { headers: {} },
					res: {
						getHeader: (k: string) => headersMap.get(k.toLowerCase()),
						setHeader: (k: string, v: any) => headersMap.set(k.toLowerCase(), v)
					}
				}
			} as any

			setUserClustersCookie(mockEvent, [{ key: 'patrolli', name: 'patrolli' }])

			const setCookieHeader = String(headersMap.get('set-cookie'))
			expect(setCookieHeader).toContain('user_clusters=')
			expect(setCookieHeader).toContain('Secure')
		})
	})

	describe('AppWebSocket connection lifecycle & race condition guard', () => {
		it('should abort superseded connect() attempt when reconnect() is called while ensureValidAccessToken is in-flight', async () => {
			setActivePinia(createPinia())
			const authStore = useAuthStore()

			const resolvers: Array<() => void> = []
			vi.spyOn(authStore, 'ensureValidAccessToken').mockImplementation(
				() =>
					new Promise((resolve) => {
						resolvers.push(() => resolve(true))
					})
			)

			const createdSockets: any[] = []
			const OriginalWebSocket = globalThis.WebSocket
			class MockWebSocket {
				static OPEN = 1
				static CLOSED = 3
				readyState = 1
				onopen: (() => void) | null = null
				onmessage: ((ev: any) => void) | null = null
				onerror: (() => void) | null = null
				onclose: ((ev: any) => void) | null = null
				close = vi.fn()
				send = vi.fn()
				constructor(
					public url: string,
					public protocols?: string | string[]
				) {
					createdSockets.push(this)
				}
			}
			globalThis.WebSocket = MockWebSocket as any

			try {
				const appWs = new AppWebSocket('ws://localhost:3000/ws/test')
				// Initial connect() is awaiting fetchCurrentUser; trigger reconnect() before it resolves
				appWs.reconnect()

				expect(resolvers).toHaveLength(2)
				// Resolve both in-flight fetchCurrentUser promises
				resolvers[0]?.()
				resolvers[1]?.()
				await Promise.resolve()
				await Promise.resolve()

				// Only the latest connect() attempt should instantiate a WebSocket
				expect(createdSockets).toHaveLength(1)
			} finally {
				globalThis.WebSocket = OriginalWebSocket
			}
		})

		it('should cancel pending backoff reconnect timeout when close() is called', async () => {
			vi.useFakeTimers()
			setActivePinia(createPinia())
			const authStore = useAuthStore()
			vi.spyOn(authStore, 'ensureValidAccessToken').mockResolvedValue(true)

			const createdSockets: any[] = []
			const OriginalWebSocket = globalThis.WebSocket
			class MockWebSocket {
				static OPEN = 1
				static CLOSED = 3
				readyState = 1
				onopen: (() => void) | null = null
				onmessage: ((ev: any) => void) | null = null
				onerror: (() => void) | null = null
				onclose: ((ev: any) => void) | null = null
				close = vi.fn()
				send = vi.fn()
				constructor(
					public url: string,
					public protocols?: string | string[]
				) {
					createdSockets.push(this)
				}
			}
			globalThis.WebSocket = MockWebSocket as any

			try {
				const appWs = new AppWebSocket('ws://localhost:3000/ws/test', {
					reconnectInterval: 1000
				})
				await Promise.resolve()
				await Promise.resolve()
				expect(createdSockets).toHaveLength(1)

				// Simulate unexpected socket drop, which schedules a 1000ms backoff timer
				createdSockets[0].onclose?.({ code: 1006, reason: 'lost', wasClean: false })

				// Explicitly close before the 1000ms timer fires
				appWs.close()
				await vi.advanceTimersByTimeAsync(2000)

				// No second socket should be created
				expect(createdSockets).toHaveLength(1)
			} finally {
				vi.useRealTimers()
				globalThis.WebSocket = OriginalWebSocket
			}
		})
	})

	describe('resolveTlsConnectOptions', () => {
		it('merges and deduplicates CA files and respects reject_unauthorized', () => {
			const caFile1 = path.resolve(process.cwd(), 'tests/scratch-ca-1.pem')
			const caFile2 = path.resolve(process.cwd(), 'tests/scratch-ca-2.pem')
			const dummyConfig = path.resolve(process.cwd(), 'tests/scratch-tls-opts.yaml')

			fs.writeFileSync(
				caFile1,
				'-----BEGIN CERTIFICATE-----\nCA_ONE\n-----END CERTIFICATE-----'
			)
			fs.writeFileSync(
				caFile2,
				'-----BEGIN CERTIFICATE-----\nCA_TWO\n-----END CERTIFICATE-----'
			)
			fs.writeFileSync(dummyConfig, `ssl:\n  ca: ${caFile1}\n  reject_unauthorized: true\n`)

			try {
				process.env.CONFIG_FILE = dummyConfig
				const opts = resolveTlsConnectOptions([caFile1, caFile2, undefined])
				expect(opts.rejectUnauthorized).toBe(true)
				expect(opts.ca).toHaveLength(tls.rootCertificates.length + 2)
				expect(opts.ca?.[opts.ca.length - 2]).toContain('CA_ONE')
				expect(opts.ca?.[opts.ca.length - 1]).toContain('CA_TWO')
			} finally {
				if (fs.existsSync(caFile1)) fs.unlinkSync(caFile1)
				if (fs.existsSync(caFile2)) fs.unlinkSync(caFile2)
				if (fs.existsSync(dummyConfig)) fs.unlinkSync(dummyConfig)
			}
		})

		it('caches CA PEM reads for unchanged files and invalidates when CA content changes', () => {
			clearCaCertCache()
			const caFile = path.resolve(process.cwd(), 'tests/scratch-cached-ca.pem')
			const dummyConfig = path.resolve(process.cwd(), 'tests/scratch-cached-ca-cfg.yaml')

			fs.writeFileSync(
				caFile,
				'-----BEGIN CERTIFICATE-----\nCA_V1\n-----END CERTIFICATE-----'
			)
			fs.writeFileSync(dummyConfig, `ssl:\n  ca: ${caFile}\n  reject_unauthorized: true\n`)

			const readSpy = vi.spyOn(fs, 'readFileSync')
			try {
				process.env.CONFIG_FILE = dummyConfig
				const first = resolveTlsConnectOptions()
				const second = resolveTlsConnectOptions()
				expect(first.ca?.[first.ca.length - 1]).toContain('CA_V1')
				expect(second.ca?.[second.ca.length - 1]).toContain('CA_V1')
				const caReadsAfterSecond = readSpy.mock.calls.filter((c) => c[0] === caFile).length
				expect(caReadsAfterSecond).toBe(1)

				fs.writeFileSync(
					caFile,
					'-----BEGIN CERTIFICATE-----\nCA_V2_UPDATED\n-----END CERTIFICATE-----'
				)
				const third = resolveTlsConnectOptions()
				expect(third.ca?.[third.ca.length - 1]).toContain('CA_V2_UPDATED')
				const caReadsAfterThird = readSpy.mock.calls.filter((c) => c[0] === caFile).length
				expect(caReadsAfterThird).toBe(2)
			} finally {
				readSpy.mockRestore()
				clearCaCertCache()
				if (fs.existsSync(caFile)) fs.unlinkSync(caFile)
				if (fs.existsSync(dummyConfig)) fs.unlinkSync(dummyConfig)
			}
		})

		it('throttles statSync calls in getTlsConnectCacheKey within statTtlMs window', () => {
			clearCaCertCache()
			const caFile = path.resolve(process.cwd(), 'tests/scratch-ttl-ca.pem')
			const dummyConfig = path.resolve(process.cwd(), 'tests/scratch-ttl-ca-cfg.yaml')

			fs.writeFileSync(
				caFile,
				'-----BEGIN CERTIFICATE-----\nCA_TTL\n-----END CERTIFICATE-----'
			)
			fs.writeFileSync(dummyConfig, `ssl:\n  ca: ${caFile}\n  reject_unauthorized: true\n`)

			const statSpy = vi.spyOn(fs, 'statSync')
			try {
				process.env.CONFIG_FILE = dummyConfig
				const key1 = sslService.getTlsConnectCacheKey([], 5000)
				const key2 = sslService.getTlsConnectCacheKey([], 5000)
				expect(key1).toBe(key2)
				const caStats = statSpy.mock.calls.filter((c) => c[0] === caFile).length
				expect(caStats).toBe(1)

				clearCaCertCache()
				sslService.getTlsConnectCacheKey([], 5000)
				const caStatsAfterClear = statSpy.mock.calls.filter((c) => c[0] === caFile).length
				expect(caStatsAfterClear).toBe(2)
			} finally {
				statSpy.mockRestore()
				clearCaCertCache()
				if (fs.existsSync(caFile)) fs.unlinkSync(caFile)
				if (fs.existsSync(dummyConfig)) fs.unlinkSync(dummyConfig)
			}
		})

		it('evicts cached CA entry when a previously cached CA file is deleted', () => {
			clearCaCertCache()
			const caFile = path.resolve(process.cwd(), 'tests/scratch-evict-ca.pem')
			const dummyConfig = path.resolve(process.cwd(), 'tests/scratch-evict-ca-cfg.yaml')

			fs.writeFileSync(
				caFile,
				'-----BEGIN CERTIFICATE-----\nCA_EVICT\n-----END CERTIFICATE-----'
			)
			fs.writeFileSync(dummyConfig, `ssl:\n  ca: ${caFile}\n  reject_unauthorized: true\n`)

			try {
				process.env.CONFIG_FILE = dummyConfig
				const opts1 = sslService.resolveTlsConnectOptions()
				expect(opts1.ca).toBeDefined()
				expect(opts1.ca?.some((c) => c.includes('CA_EVICT'))).toBe(true)

				fs.unlinkSync(caFile)
				const opts2 = sslService.resolveTlsConnectOptions()
				expect(opts2.ca).toBeUndefined()
			} finally {
				clearCaCertCache()
				if (fs.existsSync(caFile)) fs.unlinkSync(caFile)
				if (fs.existsSync(dummyConfig)) fs.unlinkSync(dummyConfig)
			}
		})
	})

	describe('getServerHttpClient TLS cache invalidation', () => {
		it('reuses client when TLS key is unchanged and rotates httpsAgent while draining idle sockets on key change', () => {
			clearServerHttpClientCache()
			const keySpy = vi.spyOn(sslService, 'getTlsConnectCacheKey')
			try {
				keySpy.mockReturnValue('true|/ca.pem:100:50')
				const client1 = getServerHttpClient()
				const client2 = getServerHttpClient()
				expect(client1).toBe(client2)

				const oldHttpsAgent = client1.defaults.httpsAgent
				const idleSocketDestroy = vi.fn()
				;(oldHttpsAgent.freeSockets as any)['cluster.local:443'] = [
					{ destroy: idleSocketDestroy }
				]

				keySpy.mockReturnValue('true|/ca.pem:200:60')
				const client3 = getServerHttpClient()
				expect(client3).not.toBe(client1)
				expect(client3.defaults.httpsAgent).not.toBe(oldHttpsAgent)
				expect(idleSocketDestroy).toHaveBeenCalledTimes(1)
			} finally {
				keySpy.mockRestore()
				clearServerHttpClientCache()
			}
		})

		it('uses O(1) TTL fast-path in getServerHttpClient without recomputing TLS key within checkTtlMs', () => {
			clearServerHttpClientCache()
			const keySpy = vi.spyOn(sslService, 'getTlsConnectCacheKey')
			try {
				keySpy.mockReturnValue('true|/ca.pem:100:50')
				const client1 = getServerHttpClient(5000)
				const client2 = getServerHttpClient(5000)
				expect(client1).toBe(client2)
				expect(keySpy).toHaveBeenCalledTimes(1)

				clearServerHttpClientCache()
				getServerHttpClient(5000)
				expect(keySpy).toHaveBeenCalledTimes(2)
			} finally {
				keySpy.mockRestore()
				clearServerHttpClientCache()
			}
		})
	})
})

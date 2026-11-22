// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import fs from 'node:fs'
import path from 'node:path'
import * as apiBaseService from '@@/server/services/apibase.service'
import {
	forwardToCluster,
	getAppConfigFileContent,
	getClustersConfig,
	getKeycloakConfig,
	getOidcConfig,
	parseClustersConfig,
	resetConfigWarnings,
	resolveClusterTargets
} from '@@/server/services/cluster.service'
import {
	clearConfigFileCache,
	readCachedYamlConfigFile
} from '@@/server/services/config-file.service'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

describe('Cluster Config Parser', () => {
	const originalEnv = process.env
	const exampleConfigPath = path.resolve(process.cwd(), 'config-web.example.yaml')

	beforeEach(() => {
		process.env = { ...originalEnv }
		delete process.env.CONFIG_FILE
	})

	afterEach(() => {
		process.env = originalEnv
	})

	it('should parse array of objects with name and url', () => {
		const input = [
			{ name: 'patrolli1', url: 'http://patrolli1:8080' },
			{ name: 'patrolli2', url: 'http://patrolli2:8080' }
		]
		const result = parseClustersConfig(input)
		expect(result).toEqual([
			{
				key: 'patrolli1',
				name: 'patrolli1',
				url: 'http://patrolli1:8080',
				clientPath: '/patrolli1'
			},
			{
				key: 'patrolli2',
				name: 'patrolli2',
				url: 'http://patrolli2:8080',
				clientPath: '/patrolli2'
			}
		])
	})

	it('should parse YAML multiline string list of clusters', () => {
		const yamlInput = `
- name: 'patrolli1'
  url: 'http://patrolli1:8080'
- name: 'patrolli2'
  url: 'http://patrolli2:8080'
`
		const result = parseClustersConfig(yamlInput)
		expect(result).toHaveLength(2)
		expect(result[0].name).toBe('patrolli1')
		expect(result[0].url).toBe('http://patrolli1:8080')
		expect(result[1].name).toBe('patrolli2')
		expect(result[1].url).toBe('http://patrolli2:8080')
	})

	it('should parse YAML multiline string with clusters key or - clusters list', () => {
		const yamlWithClusters = `
- clusters:
    - name: 'patrolli1'
      url: 'http://patrolli1:8080'
    - name: 'patrolli2'
      url: 'http://patrolli2:8080'
`
		const result = parseClustersConfig(yamlWithClusters)
		expect(result).toHaveLength(2)
		expect(result[0].name).toBe('patrolli1')
		expect(result[0].url).toBe('http://patrolli1:8080')
		expect(result[1].name).toBe('patrolli2')
		expect(result[1].url).toBe('http://patrolli2:8080')
	})

	it('should parse JSON string of clusters', () => {
		const jsonInput = JSON.stringify([
			{ name: 'patrolli1', url: 'http://patrolli1:8080' },
			{ name: 'patrolli2', url: 'http://patrolli2:8080' }
		])
		const result = parseClustersConfig(jsonInput)
		expect(result).toHaveLength(2)
		expect(result[0].name).toBe('patrolli1')
		expect(result[1].name).toBe('patrolli2')
	})

	it('should read from config-web.example.yaml file if present', () => {
		process.env.CONFIG_FILE = exampleConfigPath
		const clusters = getClustersConfig()
		expect(clusters.length).toBeGreaterThanOrEqual(1)
		expect(clusters.some((c) => c.name === 'patrolli')).toBe(true)
	})

	it('should read from custom CONFIG_FILE environment variable', () => {
		const customConfigPath = path.resolve(process.cwd(), 'custom-config-test.yaml')
		const customYaml = `
clusters:
  - name: 'custom-file-node'
    url: 'http://custom-file:8080'
`
		const existsSpy = vi.spyOn(fs, 'existsSync').mockImplementation((p) => {
			if (p === customConfigPath) return true
			return false
		})
		const readSpy = vi.spyOn(fs, 'readFileSync').mockImplementation((p, ...args) => {
			if (p === customConfigPath) return customYaml
			return (fs.readFileSync as any).getMockImplementation()?.(p, ...args)
		})

		try {
			process.env.CONFIG_FILE = customConfigPath
			const clusters = getClustersConfig()
			expect(clusters).toHaveLength(1)
			expect(clusters[0]).toMatchObject({
				name: 'custom-file-node',
				url: 'http://custom-file:8080',
				clientPath: '/custom-file-node'
			})
		} finally {
			existsSpy.mockRestore()
			readSpy.mockRestore()
		}
	})

	it('should throw an error when config file is not found', () => {
		process.env.CONFIG_FILE = path.resolve(process.cwd(), 'non-existent-config.yaml')
		expect(() => getClustersConfig()).toThrow('Configuration file not found or empty')
	})

	it('should read oidc configuration from config-web.example.yaml', () => {
		process.env.CONFIG_FILE = exampleConfigPath
		const oidc = getOidcConfig()
		expect(oidc).not.toBeNull()
		expect(oidc?.issuer).toBe('https://kc.localtest.me:8443/realms/patrolli')
		expect(oidc?.client_id).toBe('patrolli-ssr')
		expect(oidc?.redirect_uri).toContain('/auth/oidc/callback')
		expect(oidc?.token_source).toBe('access_token')
		expect(oidc?.display_name).toBe('SSO')
		expect(oidc?.ca_cert_path).toBe('./certs/ca.crt')
	})

	it('should cache YAML config reads for unchanged files and invalidate on file change', () => {
		clearConfigFileCache()
		const scratchConfig = path.resolve(process.cwd(), 'tests/scratch-cached-config.yaml')
		fs.writeFileSync(
			scratchConfig,
			'clusters:\n  - name: c1\n    url: http://c1:8080\n',
			'utf-8'
		)

		const readSpy = vi.spyOn(fs, 'readFileSync')
		try {
			const first = getAppConfigFileContent(scratchConfig)
			const second = getAppConfigFileContent(scratchConfig)
			expect(first.parsed).toEqual(second.parsed)
			expect(readSpy).toHaveBeenCalledTimes(1)

			fs.writeFileSync(
				scratchConfig,
				'clusters:\n  - name: c1\n    url: http://c1:8080\n  - name: c2\n    url: http://c2:8080\n',
				'utf-8'
			)
			const third = getAppConfigFileContent(scratchConfig)
			expect(third.parsed.clusters).toHaveLength(2)
			expect(readSpy).toHaveBeenCalledTimes(2)
		} finally {
			readSpy.mockRestore()
			clearConfigFileCache()
			if (fs.existsSync(scratchConfig)) fs.unlinkSync(scratchConfig)
		}
	})

	it('should throttle statSync calls within statTtlMs window and re-stat after cache clear', () => {
		clearConfigFileCache()
		const scratchConfig = path.resolve(process.cwd(), 'tests/scratch-ttl-config.yaml')
		fs.writeFileSync(
			scratchConfig,
			'clusters:\n  - name: c1\n    url: http://c1:8080\n',
			'utf-8'
		)

		const statSpy = vi.spyOn(fs, 'statSync')
		try {
			const first = readCachedYamlConfigFile(scratchConfig, 5000)
			const second = readCachedYamlConfigFile(scratchConfig, 5000)
			expect(first?.parsed).toEqual(second?.parsed)
			expect(statSpy.mock.calls.filter((c) => c[0] === scratchConfig)).toHaveLength(1)

			clearConfigFileCache()
			readCachedYamlConfigFile(scratchConfig, 5000)
			expect(statSpy.mock.calls.filter((c) => c[0] === scratchConfig)).toHaveLength(2)
		} finally {
			statSpy.mockRestore()
			clearConfigFileCache()
			if (fs.existsSync(scratchConfig)) fs.unlinkSync(scratchConfig)
		}
	})

	it('should evict cached YAML entry when file is deleted or emptied after TTL expiry', () => {
		clearConfigFileCache()
		const scratchConfig = path.resolve(process.cwd(), 'tests/scratch-evict-config.yaml')
		fs.writeFileSync(
			scratchConfig,
			'clusters:\n  - name: c1\n    url: http://c1:8080\n',
			'utf-8'
		)

		try {
			expect(readCachedYamlConfigFile(scratchConfig, 0)?.parsed?.clusters).toHaveLength(1)

			// Emptying the file evicts the cache entry and returns null
			fs.writeFileSync(scratchConfig, '   \n', 'utf-8')
			expect(readCachedYamlConfigFile(scratchConfig, 0)).toBeNull()
			expect(readCachedYamlConfigFile(scratchConfig, 5000)).toBeNull()

			// Repopulating and then unlinking the file also evicts the cache entry
			fs.writeFileSync(
				scratchConfig,
				'clusters:\n  - name: c1\n    url: http://c1:8080\n',
				'utf-8'
			)
			expect(readCachedYamlConfigFile(scratchConfig, 0)?.parsed?.clusters).toHaveLength(1)
			fs.unlinkSync(scratchConfig)
			expect(readCachedYamlConfigFile(scratchConfig, 0)).toBeNull()
			expect(readCachedYamlConfigFile(scratchConfig, 5000)).toBeNull()
		} finally {
			clearConfigFileCache()
			if (fs.existsSync(scratchConfig)) fs.unlinkSync(scratchConfig)
		}
	})

	it('should memoize getClustersConfig and getOidcConfig per cached YAML snapshot and invalidate on change', () => {
		clearConfigFileCache()
		const scratchConfig = path.resolve(process.cwd(), 'tests/scratch-memo-config.yaml')
		fs.writeFileSync(
			scratchConfig,
			[
				'clusters:',
				'  - name: c1',
				'    url: http://c1:8080',
				'auth:',
				'  oidc:',
				'    issuer: https://idp.example.com',
				'    client_id: patrolli-web'
			].join('\n'),
			'utf-8'
		)

		try {
			const clusters1 = getClustersConfig(undefined, scratchConfig)
			const clusters2 = getClustersConfig(undefined, scratchConfig)
			expect(clusters1).toBe(clusters2)

			const oidc1 = getOidcConfig(scratchConfig)
			const oidc2 = getOidcConfig(scratchConfig)
			expect(oidc1).toBe(oidc2)

			fs.writeFileSync(
				scratchConfig,
				[
					'clusters:',
					'  - name: c1',
					'    url: http://c1:8080',
					'  - name: c2',
					'    url: http://c2:8080',
					'auth:',
					'  oidc:',
					'    issuer: https://idp2.example.com',
					'    client_id: patrolli-web'
				].join('\n'),
				'utf-8'
			)

			const clusters3 = getClustersConfig(undefined, scratchConfig)
			expect(clusters3).not.toBe(clusters1)
			expect(clusters3).toHaveLength(2)

			const oidc3 = getOidcConfig(scratchConfig)
			expect(oidc3).not.toBe(oidc1)
			expect(oidc3?.issuer).toBe('https://idp2.example.com')
		} finally {
			clearConfigFileCache()
			if (fs.existsSync(scratchConfig)) fs.unlinkSync(scratchConfig)
		}
	})

	it('should preserve trailing slash on auth.oidc.issuer', () => {
		const customOidcPath = path.resolve(process.cwd(), 'custom-oidc-config.yaml')
		const customYaml = [
			'clusters:',
			'  - name: patrolli',
			'    url: http://patrolli:8080',
			'auth:',
			'  oidc:',
			'    issuer: https://authentik.example.com/application/o/patrolli/',
			'    client_id: patrolli-web'
		].join('\n')

		const existsSpy = vi.spyOn(fs, 'existsSync').mockImplementation((p) => p === customOidcPath)
		const readSpy = vi.spyOn(fs, 'readFileSync').mockImplementation((p) => {
			if (p === customOidcPath) return customYaml
			return ''
		})

		try {
			process.env.CONFIG_FILE = customOidcPath
			const oidc = getOidcConfig()
			expect(oidc?.issuer).toBe('https://authentik.example.com/application/o/patrolli/')
			expect(oidc?.redirect_uri).toBeUndefined()
		} finally {
			existsSpy.mockRestore()
			readSpy.mockRestore()
		}
	})

	it('should parse token_endpoint_auth_method and warn on unrecognized values', () => {
		const validPath = path.resolve(process.cwd(), 'valid-auth-method-config.yaml')
		const invalidPath = path.resolve(process.cwd(), 'invalid-oidc-values-config.yaml')
		const validYaml = [
			'clusters:',
			'  - name: patrolli',
			'    url: http://patrolli:8080',
			'auth:',
			'  oidc:',
			'    issuer: https://idp.example.com',
			'    client_id: patrolli-web',
			'    token_endpoint_auth_method: client_secret_basic'
		].join('\n')
		const invalidYaml = [
			'clusters:',
			'  - name: patrolli',
			'    url: http://patrolli:8080',
			'auth:',
			'  oidc:',
			'    issuer: https://idp.example.com',
			'    client_id: patrolli-web',
			'    token_source: access-token',
			'    token_endpoint_auth_method: private_key_jwt'
		].join('\n')

		const existsSpy = vi
			.spyOn(fs, 'existsSync')
			.mockImplementation((p) => p === validPath || p === invalidPath)
		const readSpy = vi.spyOn(fs, 'readFileSync').mockImplementation((p) => {
			if (p === validPath) return validYaml
			if (p === invalidPath) return invalidYaml
			return ''
		})
		const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {})

		try {
			process.env.CONFIG_FILE = validPath
			const validOidc = getOidcConfig()
			expect(validOidc?.token_endpoint_auth_method).toBe('client_secret_basic')

			process.env.CONFIG_FILE = invalidPath
			const invalidOidc = getOidcConfig()
			expect(invalidOidc?.token_source).toBe('id_token')
			expect(invalidOidc?.token_endpoint_auth_method).toBeUndefined()
			expect(warnSpy).toHaveBeenCalledWith(
				expect.stringContaining('Unknown auth.oidc.token_source "access-token"')
			)
			expect(warnSpy).toHaveBeenCalledWith(
				expect.stringContaining(
					'Unknown auth.oidc.token_endpoint_auth_method "private_key_jwt"'
				)
			)
		} finally {
			existsSpy.mockRestore()
			readSpy.mockRestore()
			warnSpy.mockRestore()
		}
	})

	it('should fall back to default scopes and display_name when whitespace-only values are configured', () => {
		const wsConfigPath = path.resolve(process.cwd(), 'whitespace-oidc-config.yaml')
		const wsYaml = [
			'clusters:',
			'  - name: patrolli',
			'    url: http://patrolli:8080',
			'auth:',
			'  oidc:',
			'    issuer: https://idp.example.com',
			'    client_id: patrolli-web',
			'    scopes: "   "',
			'    display_name: "   "'
		].join('\n')

		const existsSpy = vi.spyOn(fs, 'existsSync').mockImplementation((p) => p === wsConfigPath)
		const readSpy = vi.spyOn(fs, 'readFileSync').mockImplementation((p) => {
			if (p === wsConfigPath) return wsYaml
			return ''
		})

		try {
			process.env.CONFIG_FILE = wsConfigPath
			const oidc = getOidcConfig()
			expect(oidc?.scopes).toBe('openid profile email')
			expect(oidc?.display_name).toBe('SSO')
		} finally {
			existsSpy.mockRestore()
			readSpy.mockRestore()
		}
	})

	it('should support legacy auth.keycloak config with Keycloak defaults and log deprecation warning once', () => {
		clearConfigFileCache()
		resetConfigWarnings()
		const legacyKcPath = path.resolve(process.cwd(), 'legacy-keycloak-config.yaml')
		const bothAuthPath = path.resolve(process.cwd(), 'both-oidc-keycloak-config.yaml')
		const legacyKcYaml = [
			'clusters:',
			'  - name: patrolli',
			'    url: http://patrolli:8080',
			'auth:',
			'  keycloak:',
			'    host: https://kc.localtest.me:8443/realms/patrolli///',
			'    client_id: patrolli-ssr',
			'    client_secret: legacy-secret',
			'    redirect_uri: /auth/oidc/callback',
			'    ca_cert_path: ./certs/ca.crt'
		].join('\n')
		const bothAuthYaml = [
			'clusters:',
			'  - name: patrolli',
			'    url: http://patrolli:8080',
			'auth:',
			'  oidc:',
			'    issuer: https://new-idp.example.com',
			'    client_id: new-oidc-client',
			'  keycloak:',
			'    host: https://old-kc.example.com/realms/patrolli',
			'    client_id: old-kc-client'
		].join('\n')

		const existsSpy = vi
			.spyOn(fs, 'existsSync')
			.mockImplementation((p) => p === legacyKcPath || p === bothAuthPath)
		const readSpy = vi.spyOn(fs, 'readFileSync').mockImplementation((p) => {
			if (p === legacyKcPath) return legacyKcYaml
			if (p === bothAuthPath) return bothAuthYaml
			return ''
		})
		const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {})

		try {
			process.env.CONFIG_FILE = legacyKcPath
			const oidcFromKc = getOidcConfig()
			expect(oidcFromKc).toEqual({
				issuer: 'https://kc.localtest.me:8443/realms/patrolli',
				client_id: 'patrolli-ssr',
				client_secret: 'legacy-secret',
				redirect_uri: '/auth/oidc/callback',
				scopes: 'openid profile email',
				token_source: 'access_token',
				token_endpoint_auth_method: undefined,
				display_name: 'Keycloak',
				ca_cert_path: './certs/ca.crt'
			})
			expect(getKeycloakConfig()).toEqual({
				host: 'https://kc.localtest.me:8443/realms/patrolli',
				client_id: 'patrolli-ssr',
				client_secret: 'legacy-secret',
				redirect_uri: '/auth/oidc/callback',
				ca_cert_path: './certs/ca.crt'
			})
			expect(warnSpy).toHaveBeenCalledTimes(1)
			expect(warnSpy).toHaveBeenCalledWith(
				expect.stringContaining('"auth.keycloak" is deprecated')
			)

			// Prioritize auth.oidc when both auth.oidc and auth.keycloak are present
			process.env.CONFIG_FILE = bothAuthPath
			const prioritized = getOidcConfig()
			expect(prioritized?.issuer).toBe('https://new-idp.example.com')
			expect(prioritized?.client_id).toBe('new-oidc-client')
			expect(prioritized?.token_source).toBe('id_token')
			expect(prioritized?.display_name).toBe('SSO')
		} finally {
			existsSpy.mockRestore()
			readSpy.mockRestore()
			warnSpy.mockRestore()
			resetConfigWarnings()
		}
	})

	describe('resolveClusterTargets & SSRF Protection', () => {
		const clusters = [
			{
				key: 'patrolli1',
				name: 'patrolli1',
				url: 'http://patrolli1:8080',
				clientPath: '/patrolli1'
			},
			{
				key: 'patrolli2',
				name: 'patrolli2',
				url: 'http://patrolli2:8080',
				clientPath: '/patrolli2'
			}
		]

		it('should return all clusters when header is empty or all', () => {
			expect(resolveClusterTargets(null, clusters)).toEqual(clusters)
			expect(resolveClusterTargets('', clusters)).toEqual(clusters)
			expect(resolveClusterTargets('all', clusters)).toEqual(clusters)
		})

		it('should resolve cluster by name or key', () => {
			const res = resolveClusterTargets('patrolli1', clusters)
			expect(res).toHaveLength(1)
			expect(res[0].name).toBe('patrolli1')
		})

		it('should reject unwhitelisted SSRF URLs and fallback safely to registered clusters', () => {
			// Attack attempts with AWS metadata, internal port, and external evil domain
			const attack1 = resolveClusterTargets('http://169.254.169.254', clusters)
			expect(attack1).toEqual(clusters)
			expect(attack1.some((c) => c.url.includes('169.254'))).toBe(false)

			const attack2 = resolveClusterTargets('http://127.0.0.1:22', clusters)
			expect(attack2).toEqual(clusters)
			expect(attack2.some((c) => c.url.includes(':22'))).toBe(false)

			const attack3 = resolveClusterTargets('http://evil-attacker.com', clusters)
			expect(attack3).toEqual(clusters)
			expect(attack3.some((c) => c.url.includes('evil-attacker.com'))).toBe(false)
		})
	})

	describe('forwardToCluster 1:1 upstream path forwarding', () => {
		it('should forward request to cluster with 1:1 target path', async () => {
			const reqSpy = vi.spyOn(apiBaseService, 'executeHttpRequest').mockResolvedValue({
				status: 200,
				headers: {},
				data: { success: true }
			} as any)

			const cluster = { name: 'c1', url: 'http://c1:8080' }
			const res = await forwardToCluster(cluster, '/api/v1/items', {
				method: 'GET'
			})

			expect(reqSpy).toHaveBeenCalledWith(
				expect.objectContaining({
					url: 'http://c1:8080/api/v1/items',
					method: 'GET'
				})
			)
			expect(res.status).toBe(200)
		})

		it('should preserve cluster base path when forwarding 1:1 target path', async () => {
			const reqSpy = vi.spyOn(apiBaseService, 'executeHttpRequest').mockResolvedValue({
				status: 200,
				headers: {},
				data: { success: true }
			} as any)

			const cluster = { name: 'c1', url: 'http://c1:8080/prefix' }
			const res = await forwardToCluster(cluster, '/api/v1/nodes', {
				method: 'GET'
			})

			expect(reqSpy).toHaveBeenCalledWith(
				expect.objectContaining({
					url: 'http://c1:8080/prefix/api/v1/nodes',
					method: 'GET'
				})
			)
			expect(res.status).toBe(200)
		})
	})
})

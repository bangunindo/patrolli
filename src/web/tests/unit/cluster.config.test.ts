// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import fs from 'node:fs'
import path from 'node:path'
import * as apiBaseService from '@@/server/services/apibase.service'
import {
	forwardToCluster,
	getClustersConfig,
	getKeycloakConfig,
	parseClustersConfig,
	resolveClusterTargets
} from '@@/server/services/cluster.service'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

describe('Cluster Config Parser', () => {
	const originalEnv = process.env
	const exampleConfigPath = path.resolve(process.cwd(), 'config-fe.example.yaml')

	beforeEach(() => {
		process.env = { ...originalEnv }
		delete process.env.CLUSTERS
		delete process.env.CLUSTER
		delete process.env.NUXT_CLUSTERS
		delete process.env.NUXT_CLUSTER
		delete process.env.CONFIG_FILE
		delete process.env.CONFIG_PATH
		delete process.env.FRONTEND_CONFIG
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

	it('should read from config-fe.example.yaml file if present', () => {
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

	it('should fallback to process.env.CLUSTERS when config file is not found', () => {
		process.env.CONFIG_FILE = path.resolve(process.cwd(), 'non-existent-config.yaml')
		process.env.CLUSTERS = `
- name: 'custom1'
  url: 'http://custom1:8080'
- name: 'custom2'
  url: 'http://custom2:8080'
`
		const clusters = getClustersConfig()
		expect(clusters).toHaveLength(2)
		expect(clusters[0]).toMatchObject({
			name: 'custom1',
			url: 'http://custom1:8080',
			clientPath: '/custom1'
		})
		expect(clusters[1]).toMatchObject({
			name: 'custom2',
			url: 'http://custom2:8080',
			clientPath: '/custom2'
		})
	})

	it('should fallback to process.env.CLUSTER when config file is not found', () => {
		process.env.CONFIG_FILE = path.resolve(process.cwd(), 'non-existent-config.yaml')
		process.env.CLUSTER = `
- name: 'patrolli'
  url: 'http://patrolli:8080'
`
		const clusters = getClustersConfig()
		expect(clusters).toHaveLength(1)
		expect(clusters[0]).toMatchObject({
			name: 'patrolli',
			url: 'http://patrolli:8080',
			clientPath: '/patrolli'
		})
	})

	it('should return empty array if config file and env are not set/found', () => {
		process.env.CONFIG_FILE = path.resolve(process.cwd(), 'non-existent-config.yaml')
		const clusters = getClustersConfig()
		expect(clusters).toEqual([])
	})

	it('should read keycloak configuration from config-fe.example.yaml', () => {
		process.env.CONFIG_FILE = exampleConfigPath
		const keycloak = getKeycloakConfig()
		expect(keycloak).not.toBeNull()
		expect(keycloak?.client_id).toBe('patrolli-ssr')
		expect(keycloak?.redirect_uri).toContain('/auth/oidc/callback')
		expect(keycloak?.ca_cert_path).toBe('./certs/ca.crt')
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

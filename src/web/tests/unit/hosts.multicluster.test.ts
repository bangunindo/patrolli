// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import hostsApi from '@/app/api/hosts.api'
import templatesApi from '@/app/api/templates.api'
import { httpService } from '@/app/services/http.service'
import { useAppStore } from '@/stores/app.store'
import { useHostsStore } from '@/stores/hosts.store'
import type { IHost, IHostForm, IHostFormTemplate } from '@@/shared/types/host.type'
import { createPinia, setActivePinia } from 'pinia'
import { beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@/app/services/http.service', () => ({
	httpService: {
		get: vi.fn(),
		post: vi.fn(),
		put: vi.fn(),
		delete: vi.fn()
	}
}))

vi.mock('@/app/api/templates.api', () => ({
	default: {
		getTemplates: vi.fn(),
		applyTemplate: vi.fn(),
		unapplyTemplate: vi.fn()
	}
}))

describe('Multicluster CRUD for Hosts Feature', () => {
	const cluster1 = '/patrolli-cluster-01'
	const cluster2 = '/patrolli-cluster-02'

	beforeEach(() => {
		setActivePinia(createPinia())
		vi.clearAllMocks()
	})

	describe('1. Hosts API Multicluster Routing', () => {
		it('getHosts routes to specific cluster when clusterURLs provided', async () => {
			const mockResponse = {
				success: true,
				data: {
					hosts: [{ host_id: 'host-1', name: 'Server A', cluster_url: cluster1 }],
					total: 1
				}
			}
			vi.mocked(httpService.get).mockResolvedValueOnce(mockResponse)

			const result = await hostsApi.getHosts({ limit: 10, offset: 0 }, [cluster1])

			expect(httpService.get).toHaveBeenCalledWith(
				'/api/hosts',
				{ params: { limit: 10, offset: 0 } },
				[cluster1]
			)
			expect(result).toEqual(mockResponse)
		})

		it('createHost routes POST to target cluster URL', async () => {
			const payload: IHostForm = {
				name: 'Host on Cluster 1',
				adm_status: 1,
				labels: '{}'
			}
			const mockResponse = {
				success: true,
				data: { host_id: 'host-c1-01', ...payload }
			}
			vi.mocked(httpService.post).mockResolvedValueOnce(mockResponse)

			const result = await hostsApi.createHost(payload, [cluster1])

			expect(httpService.post).toHaveBeenCalledWith('/api/hosts', payload, undefined, [
				cluster1
			])
			expect(result).toEqual(mockResponse)
		})

		it('updateHost routes PUT to target cluster URL', async () => {
			const payload: IHostForm = {
				name: 'Updated Host on Cluster 2',
				adm_status: 1,
				labels: '{}'
			}
			const mockResponse = {
				success: true,
				data: { host_id: 'host-c2-01', ...payload }
			}
			vi.mocked(httpService.put).mockResolvedValueOnce(mockResponse)

			const result = await hostsApi.updateHost('host-c2-01', payload, [cluster2])

			expect(httpService.put).toHaveBeenCalledWith(
				'/api/hosts/host-c2-01',
				payload,
				undefined,
				[cluster2]
			)
			expect(result).toEqual(mockResponse)
		})

		it('deleteHost routes DELETE to target cluster URL', async () => {
			const mockResponse = {
				success: true,
				data: { host_id: 'host-c1-01' }
			}
			vi.mocked(httpService.delete).mockResolvedValueOnce(mockResponse)

			const result = await hostsApi.deleteHost('host-c1-01', [cluster1])

			expect(httpService.delete).toHaveBeenCalledWith('/api/hosts/host-c1-01', undefined, [
				cluster1
			])
			expect(result).toEqual(mockResponse)
		})
	})

	describe('2. Hosts Store Multicluster CRUD Operations', () => {
		it('createHost on Cluster 1 creates host and refreshes list', async () => {
			const store = useHostsStore()

			const hostPayload: IHostForm = {
				name: 'Web Server Cluster 1',
				adm_status: 1,
				labels: '{}'
			}
			const templatePayload: IHostFormTemplate = {
				template_id: []
			}

			vi.spyOn(hostsApi, 'createHost').mockResolvedValueOnce({
				success: true,
				data: { host_id: 'new-host-1', ...hostPayload }
			} as any)

			vi.spyOn(hostsApi, 'getHosts').mockResolvedValueOnce({
				success: true,
				data: {
					hosts: [
						{
							host_id: 'new-host-1',
							name: 'Web Server Cluster 1',
							cluster_url: cluster1,
							cluster_name: 'patrolli-cluster-01'
						} as IHost
					],
					total: 1
				}
			} as any)

			const response = await store.createHost(hostPayload, templatePayload, [cluster1])

			expect(hostsApi.createHost).toHaveBeenCalledWith(hostPayload, [cluster1])
			expect(hostsApi.getHosts).toHaveBeenCalled()
			expect(response?.success).toBe(true)
			expect(store.hosts).toHaveLength(1)
			expect(store.hosts[0].host_id).toBe('new-host-1')
		})

		it('createHost with template links templates on target cluster', async () => {
			const store = useHostsStore()

			const hostPayload: IHostForm = {
				name: 'DB Server Cluster 2',
				adm_status: 1,
				labels: '{}'
			}
			const templatePayload: IHostFormTemplate = {
				template_id: ['tmpl-linux-base', 'tmpl-mysql']
			}

			vi.spyOn(hostsApi, 'createHost').mockResolvedValueOnce({
				success: true,
				data: { host_id: 'host-db-02', ...hostPayload }
			} as any)

			vi.mocked(templatesApi.applyTemplate).mockResolvedValueOnce({
				success: true,
				data: {}
			} as any)

			vi.spyOn(hostsApi, 'getHosts').mockResolvedValueOnce({
				success: true,
				data: {
					hosts: [
						{
							host_id: 'host-db-02',
							name: 'DB Server Cluster 2',
							cluster_url: cluster2
						} as IHost
					],
					total: 1
				}
			} as any)

			const response = await store.createHost(hostPayload, templatePayload, [cluster2])

			expect(hostsApi.createHost).toHaveBeenCalledWith(hostPayload, [cluster2])
			expect(templatesApi.applyTemplate).toHaveBeenCalledWith(
				{
					host_ids: ['host-db-02'],
					template_ids: ['tmpl-linux-base', 'tmpl-mysql']
				},
				[cluster2]
			)
			expect(response?.success).toBe(true)
		})

		it('updateHost on Cluster 2 updates host and handles template link/unlink on cluster', async () => {
			const store = useHostsStore()

			store.currentHost = {
				host_id: 'host-c2-99',
				name: 'Old Host Name',
				templates: [{ template_id: 'tmpl-existing' }] as any,
				cluster_url: cluster2
			} as IHost

			const updatePayload: IHostForm = {
				name: 'Updated Host Name',
				adm_status: 1,
				labels: '{}'
			}
			const templatePayload: IHostFormTemplate = {
				template_id: ['tmpl-existing', 'tmpl-new']
			}
			const unapplyChoices = [{ template_id: 'tmpl-to-remove', keep_items: false }]

			vi.spyOn(hostsApi, 'updateHost').mockResolvedValueOnce({
				success: true,
				data: { host_id: 'host-c2-99', ...updatePayload }
			} as any)

			vi.mocked(templatesApi.applyTemplate).mockResolvedValueOnce({
				success: true,
				data: {}
			} as any)

			vi.mocked(templatesApi.unapplyTemplate).mockResolvedValueOnce({
				success: true,
				data: {}
			} as any)

			vi.spyOn(hostsApi, 'getHosts').mockResolvedValueOnce({
				success: true,
				data: {
					hosts: [
						{
							host_id: 'host-c2-99',
							name: 'Updated Host Name',
							cluster_url: cluster2
						} as IHost
					],
					total: 1
				}
			} as any)

			const response = await store.updateHost(
				'host-c2-99',
				updatePayload,
				templatePayload,
				unapplyChoices,
				[cluster2]
			)

			expect(hostsApi.updateHost).toHaveBeenCalledWith('host-c2-99', updatePayload, [
				cluster2
			])
			expect(templatesApi.applyTemplate).toHaveBeenCalledWith(
				{
					host_ids: ['host-c2-99'],
					template_ids: ['tmpl-new']
				},
				[cluster2]
			)
			expect(templatesApi.unapplyTemplate).toHaveBeenCalledWith(
				{
					unapply_list: [
						{
							host_id: 'host-c2-99',
							template_id: 'tmpl-to-remove',
							keep_items: false
						}
					]
				},
				[cluster2]
			)
			expect(response?.success).toBe(true)
		})

		it('deleteHost on specific cluster removes host from store', async () => {
			const store = useHostsStore()
			store.hosts = [
				{ host_id: 'host-c1-01', name: 'Host 1', cluster_url: cluster1 } as IHost,
				{ host_id: 'host-c2-02', name: 'Host 2', cluster_url: cluster2 } as IHost
			]

			vi.spyOn(hostsApi, 'deleteHost').mockResolvedValueOnce({
				success: true,
				data: {}
			} as any)

			const response = await store.deleteHost('host-c1-01', [cluster1])

			expect(hostsApi.deleteHost).toHaveBeenCalledWith('host-c1-01', [cluster1])
			expect(response?.success).toBe(true)
			expect(store.hosts).toHaveLength(1)
			expect(store.hosts[0].host_id).toBe('host-c2-02')
		})

		it('fetchHosts respects activeClusterURL from global appStore', async () => {
			const appStore = useAppStore()
			const store = useHostsStore()

			appStore.activeClusterURL = cluster1

			vi.spyOn(hostsApi, 'getHosts').mockResolvedValueOnce({
				success: true,
				data: {
					hosts: [
						{
							host_id: 'host-c1-active',
							name: 'Active Cluster Host',
							cluster_url: cluster1
						} as IHost
					],
					total: 1
				}
			} as any)

			await store.fetchHosts()

			expect(hostsApi.getHosts).toHaveBeenCalledWith(store.filter)
			expect(store.hosts).toHaveLength(1)
			expect(store.hosts[0].host_id).toBe('host-c1-active')
		})
	})
})

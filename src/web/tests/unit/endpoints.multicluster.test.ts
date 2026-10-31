// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import endpointsApi from '@/app/api/endpoints.api'
import hostsApi from '@/app/api/hosts.api'
import { httpService } from '@/app/services/http.service'
import { useAppStore } from '@/stores/app.store'
import { useEndpointsStore } from '@/stores/endpoints.store'
import type { IEndpoint, IEndpointForm } from '@@/shared/types/endpoint.type'
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

describe('Multicluster CRUD for Endpoints Feature', () => {
	const cluster1 = '/patrolli-cluster-01'
	const cluster2 = '/patrolli-cluster-02'

	const hostId1 = '11111111-1111-1111-1111-111111111111'
	const hostId2 = '22222222-2222-2222-2222-222222222222'

	beforeEach(() => {
		setActivePinia(createPinia())
		vi.clearAllMocks()
	})

	describe('1. Endpoints API Multicluster Routing', () => {
		it('getEndpoints routes GET to specific cluster when clusterURLs provided', async () => {
			const mockResponse = {
				status_code: 200,
				success: true,
				data: {
					endpoints: [
						{
							endpoint_id: 'ep-1',
							name: 'SNMP Endpoint 1',
							host_id: hostId1,
							host_name: 'Host 1',
							cluster_url: cluster1
						}
					],
					total: 1
				}
			}
			vi.mocked(httpService.get).mockResolvedValueOnce(mockResponse)

			const result = await endpointsApi.getEndpoints({ limit: 10, offset: 0 }, [cluster1])

			expect(httpService.get).toHaveBeenCalledWith(
				'/api/endpoints',
				{ params: { limit: 10, offset: 0 } },
				[cluster1]
			)
			expect(result).toEqual(mockResponse)
		})

		it('createEndpoint routes POST to target cluster URL', async () => {
			const payload: IEndpointForm = {
				name: 'SNMP Router C1',
				host_id: hostId1,
				adm_status: 1,
				config: {
					type: 'snmp_v2c',
					config: {
						address: '192.168.1.1:161',
						community: 'public'
					}
				}
			}
			const mockResponse = {
				status_code: 200,
				success: true,
				data: { endpoint_id: 'ep-c1-01', ...payload }
			}
			vi.mocked(httpService.post).mockResolvedValueOnce(mockResponse)

			const result = await endpointsApi.createEndpoint(payload, [cluster1])

			expect(httpService.post).toHaveBeenCalledWith('/api/endpoints', payload, undefined, [
				cluster1
			])
			expect(result).toEqual(mockResponse)
		})

		it('updateEndpoint routes PUT to target cluster URL', async () => {
			const payload: IEndpointForm = {
				name: 'HTTP Endpoint C2',
				host_id: hostId2,
				adm_status: 1,
				config: {
					type: 'http',
					config: {
						connect_timeout: 5000,
						url: 'http://localhost:9090'
					}
				}
			}
			const mockResponse = {
				status_code: 200,
				success: true,
				data: { endpoint_id: 'ep-c2-01', ...payload }
			}
			vi.mocked(httpService.put).mockResolvedValueOnce(mockResponse)

			const result = await endpointsApi.updateEndpoint('ep-c2-01', payload, [cluster2])

			expect(httpService.put).toHaveBeenCalledWith(
				'/api/endpoints/ep-c2-01',
				payload,
				undefined,
				[cluster2]
			)
			expect(result).toEqual(mockResponse)
		})

		it('deleteEndpoint routes DELETE to target cluster URL', async () => {
			const mockResponse = {
				status_code: 200,
				success: true,
				data: { endpoint_id: 'ep-c1-01' }
			}
			vi.mocked(httpService.delete).mockResolvedValueOnce(mockResponse)

			const result = await endpointsApi.deleteEndpoint('ep-c1-01', [cluster1])

			expect(httpService.delete).toHaveBeenCalledWith('/api/endpoints/ep-c1-01', undefined, [
				cluster1
			])
			expect(result).toEqual(mockResponse)
		})
	})

	describe('2. Endpoints Store Multicluster CRUD Operations', () => {
		it('fetchHosts fetches hosts for dropdown on target cluster', async () => {
			const store = useEndpointsStore()

			vi.spyOn(hostsApi, 'getHosts').mockResolvedValueOnce({
				status_code: 200,
				success: true,
				data: {
					hosts: [
						{
							host_id: hostId1,
							name: 'Host 1 on C1',
							adm_status: 1 as const,
							labels: '{}',
							last_updated: new Date().toISOString(),
							cluster_url: cluster1
						}
					],
					total: 1
				}
			} as any)

			await store.fetchHosts([cluster1])

			expect(hostsApi.getHosts).toHaveBeenCalledWith({ limit: 100 }, [cluster1])
			expect(store.selection.hosts).toHaveLength(1)
			expect(store.selection.hosts[0].value).toBe(hostId1)
		})

		it('createEndpoint on Cluster 1 creates endpoint and refreshes list', async () => {
			const store = useEndpointsStore()

			const payload: IEndpointForm = {
				name: 'SNMP Agent C1',
				host_id: hostId1,
				adm_status: 1,
				config: {
					type: 'snmp_v2c',
					config: {
						address: '192.168.1.1:161',
						community: 'public'
					}
				}
			}

			vi.spyOn(endpointsApi, 'createEndpoint').mockResolvedValueOnce({
				status_code: 200,
				success: true,
				data: {
					endpoint_id: 'new-ep-01',
					...payload,
					last_updated: new Date().toISOString()
				} as any
			} as any)

			vi.spyOn(endpointsApi, 'getEndpoints').mockResolvedValue({
				status_code: 200,
				success: true,
				data: {
					endpoints: [
						{
							endpoint_id: 'new-ep-01',
							name: 'SNMP Agent C1',
							host_id: hostId1,
							host_name: 'Host 1',
							adm_status: 1,
							config: payload.config,
							last_updated: new Date().toISOString(),
							cluster_url: cluster1,
							cluster_name: 'patrolli-cluster-01'
						}
					],
					total: 1
				}
			} as any)

			const response = await store.createEndpoint(payload, [cluster1])

			expect(endpointsApi.createEndpoint).toHaveBeenCalledWith(payload, [cluster1])
			expect(endpointsApi.getEndpoints).toHaveBeenCalled()
			expect(response?.success).toBe(true)
			expect(store.endpoints).toHaveLength(1)
			expect(store.endpoints[0].endpoint_id).toBe('new-ep-01')
		})

		it('updateEndpoint on Cluster 2 updates endpoint and refreshes list', async () => {
			const store = useEndpointsStore()

			const updatePayload: IEndpointForm = {
				name: 'Prometheus Scrape HTTP C2',
				host_id: hostId2,
				adm_status: 1,
				config: {
					type: 'http',
					config: {
						connect_timeout: 5000,
						url: 'http://metrics.internal:9090'
					}
				}
			}

			vi.spyOn(endpointsApi, 'updateEndpoint').mockResolvedValueOnce({
				status_code: 200,
				success: true,
				data: {
					endpoint_id: 'ep-c2-01',
					...updatePayload,
					last_updated: new Date().toISOString()
				} as any
			} as any)

			vi.spyOn(endpointsApi, 'getEndpoints').mockResolvedValue({
				status_code: 200,
				success: true,
				data: {
					endpoints: [
						{
							endpoint_id: 'ep-c2-01',
							name: 'Prometheus Scrape HTTP C2',
							host_id: hostId2,
							host_name: 'Host 2',
							adm_status: 1,
							config: updatePayload.config,
							last_updated: new Date().toISOString(),
							cluster_url: cluster2,
							cluster_name: 'patrolli-cluster-02'
						}
					],
					total: 1
				}
			} as any)

			const response = await store.updateEndpoint('ep-c2-01', updatePayload, [cluster2])

			expect(endpointsApi.updateEndpoint).toHaveBeenCalledWith('ep-c2-01', updatePayload, [
				cluster2
			])
			expect(endpointsApi.getEndpoints).toHaveBeenCalled()
			expect(response?.success).toBe(true)
			expect(store.endpoints).toHaveLength(1)
			expect(store.endpoints[0].name).toBe('Prometheus Scrape HTTP C2')
		})

		it('deleteEndpoint on Cluster 1 deletes endpoint and removes from store', async () => {
			const store = useEndpointsStore()
			store.endpoints = [
				{
					endpoint_id: 'ep-c1-01',
					name: 'EP 1',
					host_id: hostId1,
					host_name: 'Host 1',
					adm_status: 1,
					cluster_url: cluster1
				} as IEndpoint,
				{
					endpoint_id: 'ep-c2-02',
					name: 'EP 2',
					host_id: hostId2,
					host_name: 'Host 2',
					adm_status: 1,
					cluster_url: cluster2
				} as IEndpoint
			]

			vi.spyOn(endpointsApi, 'deleteEndpoint').mockResolvedValueOnce({
				status_code: 200,
				success: true,
				data: {}
			} as any)

			const response = await store.deleteEndpoint('ep-c1-01', [cluster1])

			expect(endpointsApi.deleteEndpoint).toHaveBeenCalledWith('ep-c1-01', [cluster1])
			expect(response?.success).toBe(true)
			expect(store.endpoints).toHaveLength(1)
			expect(store.endpoints[0].endpoint_id).toBe('ep-c2-02')
		})

		it('fetchEndpoints respects activeClusterURL from global appStore', async () => {
			const appStore = useAppStore()
			const store = useEndpointsStore()

			appStore.activeClusterURL = cluster1

			vi.spyOn(endpointsApi, 'getEndpoints').mockResolvedValueOnce({
				status_code: 200,
				success: true,
				data: {
					endpoints: [
						{
							endpoint_id: 'ep-c1-active',
							name: 'Active Cluster Endpoint',
							host_id: hostId1,
							host_name: 'Host 1',
							adm_status: 1,
							cluster_url: cluster1
						}
					],
					total: 1
				}
			} as any)

			await store.fetchEndpoints()

			expect(endpointsApi.getEndpoints).toHaveBeenCalledWith(store.filter)
			expect(store.endpoints).toHaveLength(1)
			expect(store.endpoints[0].endpoint_id).toBe('ep-c1-active')
		})

		it('setDetailCanvas populates currentEndpoint by endpoint_id and name', async () => {
			const store = useEndpointsStore()

			const mockEndpoint = {
				endpoint_id: 'ep-target-99',
				name: 'Target Endpoint',
				host_id: hostId1,
				host_name: 'Host 1',
				adm_status: 1,
				cluster_url: cluster1
			}

			vi.spyOn(endpointsApi, 'getEndpoints').mockResolvedValueOnce({
				status_code: 200,
				success: true,
				data: {
					endpoints: [mockEndpoint],
					total: 1
				}
			} as any)

			await store.setDetailCanvas({ name: 'Target Endpoint', endpoint_id: 'ep-target-99' })

			expect(store.currentEndpoint).toEqual(mockEndpoint)
		})
	})
})

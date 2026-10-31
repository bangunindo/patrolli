// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import hostsApi from '@/app/api/hosts.api'
import listenersApi from '@/app/api/listeners.api'
import { httpService } from '@/app/services/http.service'
import { useAppStore } from '@/stores/app.store'
import { useListenerStore } from '@/stores/listeners.store'
import type { IListener, IListenerForm } from '@@/shared/types/listener.type'
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

describe('Multicluster CRUD for Listeners Feature', () => {
	const cluster1 = '/patrolli-cluster-01'
	const cluster2 = '/patrolli-cluster-02'

	beforeEach(() => {
		setActivePinia(createPinia())
		vi.clearAllMocks()
	})

	describe('1. Listeners API Multicluster Routing', () => {
		it('getListeners routes GET to specific cluster when clusterURLs provided', async () => {
			const mockResponse = {
				status_code: 200,
				success: true,
				data: {
					listeners: [
						{
							listener_id: 'lis-1',
							name: 'Syslog Listener C1',
							adm_status: 1,
							labels: '{}',
							worker_count: 1,
							disable_cluster_forwarding: false,
							last_updated: new Date().toISOString(),
							cluster_url: cluster1
						}
					],
					total: 1
				}
			}
			vi.mocked(httpService.get).mockResolvedValueOnce(mockResponse)

			const result = await listenersApi.getListeners({ limit: 10, offset: 0 }, [cluster1])

			expect(httpService.get).toHaveBeenCalledWith(
				'/api/listeners',
				{ params: { limit: 10, offset: 0 } },
				[cluster1]
			)
			expect(result).toEqual(mockResponse)
		})

		it('createListener routes POST to target cluster URL', async () => {
			const payload: IListenerForm = {
				name: 'Syslog UDP C1',
				adm_status: 1,
				labels: '{}',
				config: {
					type: 'syslog',
					config: {
						listener: {
							udp: {
								address: '0.0.0.0:514'
							}
						}
					}
				}
			}
			const mockResponse = {
				status_code: 200,
				success: true,
				data: { listener_id: 'lis-c1-01', ...payload }
			}
			vi.mocked(httpService.post).mockResolvedValueOnce(mockResponse)

			const result = await listenersApi.createListener(payload, [cluster1])

			expect(httpService.post).toHaveBeenCalledWith('/api/listeners', payload, undefined, [
				cluster1
			])
			expect(result).toEqual(mockResponse)
		})

		it('updateListener routes PUT to target cluster URL', async () => {
			const payload: IListenerForm = {
				name: 'Syslog Updated C2',
				adm_status: 1,
				labels: '{}',
				config: {
					type: 'dummy',
					config: {
						messages_per_batch: 10,
						speed: 5
					}
				}
			}
			const mockResponse = {
				status_code: 200,
				success: true,
				data: { listener_id: 'lis-c2-01', ...payload }
			}
			vi.mocked(httpService.put).mockResolvedValueOnce(mockResponse)

			const result = await listenersApi.updateListener('lis-c2-01', payload, [cluster2])

			expect(httpService.put).toHaveBeenCalledWith(
				'/api/listeners/lis-c2-01',
				payload,
				undefined,
				[cluster2]
			)
			expect(result).toEqual(mockResponse)
		})

		it('deleteListener routes DELETE to target cluster URL', async () => {
			const mockResponse = {
				status_code: 200,
				success: true,
				data: { listener_id: 'lis-c1-01' }
			}
			vi.mocked(httpService.delete).mockResolvedValueOnce(mockResponse)

			const result = await listenersApi.deleteListener('lis-c1-01', [cluster1])

			expect(httpService.delete).toHaveBeenCalledWith('/api/listeners/lis-c1-01', undefined, [
				cluster1
			])
			expect(result).toEqual(mockResponse)
		})
	})

	describe('2. Listeners Store Multicluster CRUD Operations', () => {
		it('fetchHosts fetches hosts for dropdown on target cluster', async () => {
			const store = useListenerStore()

			vi.spyOn(hostsApi, 'getHosts').mockResolvedValueOnce({
				status_code: 200,
				success: true,
				data: {
					hosts: [
						{
							host_id: 'host-1',
							name: 'Host 1',
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
			expect(store.selection.hosts[0].value).toBe('host-1')
		})

		it('createListener on Cluster 1 creates listener and refreshes list', async () => {
			const store = useListenerStore()

			const payload: IListenerForm = {
				name: 'Syslog UDP C1',
				adm_status: 1,
				labels: '{}',
				config: {
					type: 'syslog',
					config: {
						listener: {
							udp: {
								address: '0.0.0.0:514'
							}
						}
					}
				}
			}

			vi.spyOn(listenersApi, 'createListener').mockResolvedValueOnce({
				status_code: 200,
				success: true,
				data: {
					listener_id: 'new-lis-01',
					...payload,
					worker_count: 1,
					disable_cluster_forwarding: false,
					last_updated: new Date().toISOString()
				} as any
			} as any)

			vi.spyOn(listenersApi, 'getListeners').mockResolvedValue({
				status_code: 200,
				success: true,
				data: {
					listeners: [
						{
							listener_id: 'new-lis-01',
							name: 'Syslog UDP C1',
							adm_status: 1,
							labels: '{}',
							worker_count: 1,
							config: payload.config,
							disable_cluster_forwarding: false,
							last_updated: new Date().toISOString(),
							cluster_url: cluster1,
							cluster_name: 'patrolli-cluster-01'
						}
					],
					total: 1
				}
			} as any)

			const response = await store.createListener(payload, [cluster1])

			expect(listenersApi.createListener).toHaveBeenCalledWith(payload, [cluster1])
			expect(listenersApi.getListeners).toHaveBeenCalled()
			expect(response?.success).toBe(true)
			expect(store.listeners).toHaveLength(1)
			expect(store.listeners[0].listener_id).toBe('new-lis-01')
		})

		it('updateListener on Cluster 2 updates listener and refreshes list', async () => {
			const store = useListenerStore()

			const updatePayload: IListenerForm = {
				name: 'Dummy Generator C2',
				adm_status: 1,
				labels: '{}',
				config: {
					type: 'dummy',
					config: {
						messages_per_batch: 20,
						speed: 10
					}
				}
			}

			vi.spyOn(listenersApi, 'updateListener').mockResolvedValueOnce({
				status_code: 200,
				success: true,
				data: {
					listener_id: 'lis-c2-01',
					...updatePayload,
					worker_count: 1,
					disable_cluster_forwarding: false,
					last_updated: new Date().toISOString()
				} as any
			} as any)

			vi.spyOn(listenersApi, 'getListeners').mockResolvedValue({
				status_code: 200,
				success: true,
				data: {
					listeners: [
						{
							listener_id: 'lis-c2-01',
							name: 'Dummy Generator C2',
							adm_status: 1,
							labels: '{}',
							worker_count: 1,
							config: updatePayload.config,
							disable_cluster_forwarding: false,
							last_updated: new Date().toISOString(),
							cluster_url: cluster2,
							cluster_name: 'patrolli-cluster-02'
						}
					],
					total: 1
				}
			} as any)

			const response = await store.updateListener('lis-c2-01', updatePayload, [cluster2])

			expect(listenersApi.updateListener).toHaveBeenCalledWith('lis-c2-01', updatePayload, [
				cluster2
			])
			expect(listenersApi.getListeners).toHaveBeenCalled()
			expect(response?.success).toBe(true)
			expect(store.listeners).toHaveLength(1)
			expect(store.listeners[0].name).toBe('Dummy Generator C2')
		})

		it('deleteListener on Cluster 1 deletes listener and removes from store', async () => {
			const store = useListenerStore()
			store.listeners = [
				{
					listener_id: 'lis-c1-01',
					name: 'L1',
					adm_status: 1,
					labels: '{}',
					worker_count: 1,
					disable_cluster_forwarding: false,
					last_updated: new Date().toISOString(),
					config: { type: 'dummy', config: { messages_per_batch: 1, speed: 1 } },
					cluster_url: cluster1
				} as IListener,
				{
					listener_id: 'lis-c2-02',
					name: 'L2',
					adm_status: 1,
					labels: '{}',
					worker_count: 1,
					disable_cluster_forwarding: false,
					last_updated: new Date().toISOString(),
					config: { type: 'dummy', config: { messages_per_batch: 1, speed: 1 } },
					cluster_url: cluster2
				} as IListener
			]

			vi.spyOn(listenersApi, 'deleteListener').mockResolvedValueOnce({
				status_code: 200,
				success: true,
				data: {}
			} as any)

			const response = await store.deleteListener('lis-c1-01', [cluster1])

			expect(listenersApi.deleteListener).toHaveBeenCalledWith('lis-c1-01', [cluster1])
			expect(response?.success).toBe(true)
			expect(store.listeners).toHaveLength(1)
			expect(store.listeners[0].listener_id).toBe('lis-c2-02')
		})

		it('fetchListeners respects activeClusterURL from global appStore', async () => {
			const appStore = useAppStore()
			const store = useListenerStore()

			appStore.activeClusterURL = cluster1

			vi.spyOn(listenersApi, 'getListeners').mockResolvedValueOnce({
				status_code: 200,
				success: true,
				data: {
					listeners: [
						{
							listener_id: 'lis-c1-active',
							name: 'Active Cluster Listener',
							adm_status: 1,
							labels: '{}',
							worker_count: 1,
							disable_cluster_forwarding: false,
							last_updated: new Date().toISOString(),
							config: { type: 'dummy', config: { messages_per_batch: 1, speed: 1 } },
							cluster_url: cluster1
						}
					],
					total: 1
				}
			} as any)

			await store.fetchListeners()

			expect(listenersApi.getListeners).toHaveBeenCalledWith(store.filter)
			expect(store.listeners).toHaveLength(1)
			expect(store.listeners[0].listener_id).toBe('lis-c1-active')
		})

		it('setDetailCanvas populates currentListener by listener_id and name', async () => {
			const store = useListenerStore()

			const mockListener = {
				listener_id: 'lis-target-99',
				name: 'Target Listener',
				adm_status: 1,
				labels: '{}',
				worker_count: 1,
				disable_cluster_forwarding: false,
				last_updated: new Date().toISOString(),
				config: { type: 'dummy', config: { messages_per_batch: 1, speed: 1 } },
				cluster_url: cluster1
			}

			vi.spyOn(listenersApi, 'getListeners').mockResolvedValueOnce({
				status_code: 200,
				success: true,
				data: {
					listeners: [mockListener],
					total: 1
				}
			} as any)

			await store.setDetailCanvas({ name: 'Target Listener', listener_id: 'lis-target-99' })

			expect(store.currentListener).toEqual(mockListener)
		})
	})
})

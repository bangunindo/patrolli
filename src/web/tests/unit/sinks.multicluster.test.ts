// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import sinksApi from '@/app/api/sinks.api'
import { httpService } from '@/app/services/http.service'
import { useAppStore } from '@/stores/app.store'
import { useSinksStore } from '@/stores/sinks.store'
import type { ISinkForm } from '@@/shared/types/sink.type'
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

describe('Multicluster CRUD for Sinks Feature', () => {
	const cluster1 = '/patrolli-cluster-01'
	const cluster2 = '/patrolli-cluster-02'

	beforeEach(() => {
		setActivePinia(createPinia())
		vi.clearAllMocks()
	})

	describe('1. Sinks API Multicluster Routing', () => {
		it('getSinks routes GET to /api/sinks on target cluster URL', async () => {
			const mockResponse = {
				status_code: 200,
				success: true,
				data: {
					sinks: [
						{
							sink_id: 'sink-1',
							name: 'File Sink 1',
							adm_status: 1,
							input_count: 0,
							worker_count: 1,
							labels: '{}',
							batch: { batch_size: 1000, batch_timeout: 1000 },
							channel: { type: 'memory' as const },
							config: { type: 'file' as const, config: { path: '/tmp/output.log' } },
							last_updated: new Date().toISOString(),
							cluster_url: cluster1
						}
					],
					total: 1
				}
			}
			vi.mocked(httpService.get).mockResolvedValueOnce(mockResponse)

			const result = await sinksApi.getSinks({ limit: 10, offset: 0 }, [cluster1])

			expect(httpService.get).toHaveBeenCalledWith(
				'/api/sinks',
				{ params: { limit: 10, offset: 0 } },
				[cluster1]
			)
			expect(result).toEqual(mockResponse)
		})

		it('createSink routes POST to /api/sinks on target cluster URL', async () => {
			const payload: ISinkForm = {
				name: 'File Logger C1',
				adm_status: 1,
				labels: '{}',
				inputs: [],
				batch: {
					batch_size: 1000,
					batch_timeout: 1000
				},
				channel: {
					type: 'memory',
					non_blocking: false
				},
				config: {
					type: 'file',
					config: {
						path: '/var/log/patrolli/c1.log'
					}
				}
			}
			const mockResponse = {
				status_code: 200,
				success: true,
				data: { sink_id: 'sink-c1-01', ...payload }
			}
			vi.mocked(httpService.post).mockResolvedValueOnce(mockResponse)

			const result = await sinksApi.createSink(payload, [cluster1])

			expect(httpService.post).toHaveBeenCalledWith('/api/sinks', payload, undefined, [
				cluster1
			])
			expect(result).toEqual(mockResponse)
		})

		it('getSinkById routes GET to /api/sinks/:id on target cluster URL', async () => {
			const mockResponse = {
				status_code: 200,
				success: true,
				data: {
					sink_id: 'sink-c1-01',
					name: 'File Logger C1',
					cluster_url: cluster1
				}
			}
			vi.mocked(httpService.get).mockResolvedValueOnce(mockResponse)

			const result = await sinksApi.getSinkById('sink-c1-01', [cluster1])

			expect(httpService.get).toHaveBeenCalledWith('/api/sinks/sink-c1-01', undefined, [
				cluster1
			])
			expect(result).toEqual(mockResponse)
		})

		it('updateSink routes PUT to /api/sinks/:id on target cluster URL', async () => {
			const payload: ISinkForm = {
				name: 'Updated File Logger C2',
				adm_status: 1,
				labels: '{}',
				inputs: [],
				batch: {
					batch_size: 2000,
					batch_timeout: 500
				},
				channel: {
					type: 'memory'
				},
				config: {
					type: 'file',
					config: {
						path: '/var/log/patrolli/c2_updated.log'
					}
				}
			}
			const mockResponse = {
				status_code: 200,
				success: true,
				data: { sink_id: 'sink-c2-01', ...payload }
			}
			vi.mocked(httpService.put).mockResolvedValueOnce(mockResponse)

			const result = await sinksApi.updateSink('sink-c2-01', payload, [cluster2])

			expect(httpService.put).toHaveBeenCalledWith(
				'/api/sinks/sink-c2-01',
				payload,
				undefined,
				[cluster2]
			)
			expect(result).toEqual(mockResponse)
		})

		it('deleteSink routes DELETE to /api/sinks/:id on target cluster URL', async () => {
			const mockResponse = {
				status_code: 200,
				success: true,
				data: { sink_id: 'sink-c1-01' }
			}
			vi.mocked(httpService.delete).mockResolvedValueOnce(mockResponse)

			const result = await sinksApi.deleteSink('sink-c1-01', [cluster1])

			expect(httpService.delete).toHaveBeenCalledWith('/api/sinks/sink-c1-01', undefined, [
				cluster1
			])
			expect(result).toEqual(mockResponse)
		})
	})

	describe('2. Sinks Store Multicluster CRUD Operations', () => {
		it('createSink on Cluster 1 creates sink and refreshes list', async () => {
			const store = useSinksStore()

			const payload: ISinkForm = {
				name: 'Disk Storage Sink C1',
				adm_status: 1,
				labels: '{}',
				inputs: [],
				batch: { batch_size: 1000, batch_timeout: 1000 },
				channel: { type: 'memory' },
				config: {
					type: 'file',
					config: { path: '/data/metrics.log' }
				}
			}

			vi.spyOn(sinksApi, 'createSink').mockResolvedValueOnce({
				status_code: 200,
				success: true,
				data: {
					sink_id: 'new-sink-01',
					...payload,
					worker_count: 1,
					last_updated: new Date().toISOString()
				} as any
			} as any)

			vi.spyOn(sinksApi, 'getSinks').mockResolvedValue({
				status_code: 200,
				success: true,
				data: {
					sinks: [
						{
							sink_id: 'new-sink-01',
							name: 'Disk Storage Sink C1',
							adm_status: 1,
							input_count: 0,
							labels: '{}',
							worker_count: 1,
							batch: payload.batch,
							channel: payload.channel,
							config: payload.config,
							last_updated: new Date().toISOString(),
							cluster_url: cluster1,
							cluster_name: 'patrolli-cluster-01'
						}
					],
					total: 1
				}
			} as any)

			const response = await store.createSink(payload, [cluster1])

			expect(sinksApi.createSink).toHaveBeenCalledWith(payload, [cluster1])
			expect(sinksApi.getSinks).toHaveBeenCalled()
			expect(response?.success).toBe(true)
			expect(store.sinks).toHaveLength(1)
			expect(store.sinks[0].sink_id).toBe('new-sink-01')
		})

		it('updateSink on Cluster 2 updates sink and refreshes list', async () => {
			const store = useSinksStore()

			const updatePayload: ISinkForm = {
				name: 'Updated Sink C2',
				adm_status: 1,
				labels: '{}',
				inputs: [],
				batch: { batch_size: 500, batch_timeout: 500 },
				channel: { type: 'memory' },
				config: {
					type: 'file',
					config: { path: '/data/c2.log' }
				}
			}

			vi.spyOn(sinksApi, 'updateSink').mockResolvedValueOnce({
				status_code: 200,
				success: true,
				data: {
					sink_id: 'sink-c2-01',
					...updatePayload,
					worker_count: 1,
					last_updated: new Date().toISOString()
				} as any
			} as any)

			vi.spyOn(sinksApi, 'getSinks').mockResolvedValue({
				status_code: 200,
				success: true,
				data: {
					sinks: [
						{
							sink_id: 'sink-c2-01',
							name: 'Updated Sink C2',
							adm_status: 1,
							input_count: 0,
							labels: '{}',
							worker_count: 1,
							batch: updatePayload.batch,
							channel: updatePayload.channel,
							config: updatePayload.config,
							last_updated: new Date().toISOString(),
							cluster_url: cluster2,
							cluster_name: 'patrolli-cluster-02'
						}
					],
					total: 1
				}
			} as any)

			const response = await store.updateSink('sink-c2-01', updatePayload, [cluster2])

			expect(sinksApi.updateSink).toHaveBeenCalledWith('sink-c2-01', updatePayload, [
				cluster2
			])
			expect(sinksApi.getSinks).toHaveBeenCalled()
			expect(response?.success).toBe(true)
			expect(store.sinks).toHaveLength(1)
			expect(store.sinks[0].name).toBe('Updated Sink C2')
		})

		it('deleteSink on Cluster 1 deletes sink and removes from store', async () => {
			const store = useSinksStore()
			store.sinks = [
				{
					sink_id: 'sink-c1-01',
					name: 'Sink 1',
					adm_status: 1,
					input_count: 0,
					labels: '{}',
					worker_count: 1,
					batch: { batch_size: 1000, batch_timeout: 1000 },
					channel: { type: 'memory' },
					config: { type: 'file', config: { path: '/tmp/1.log' } },
					last_updated: new Date().toISOString(),
					cluster_url: cluster1
				},
				{
					sink_id: 'sink-c2-02',
					name: 'Sink 2',
					adm_status: 1,
					input_count: 0,
					labels: '{}',
					worker_count: 1,
					batch: { batch_size: 1000, batch_timeout: 1000 },
					channel: { type: 'memory' },
					config: { type: 'file', config: { path: '/tmp/2.log' } },
					last_updated: new Date().toISOString(),
					cluster_url: cluster2
				}
			]

			vi.spyOn(sinksApi, 'deleteSink').mockResolvedValueOnce({
				status_code: 200,
				success: true,
				data: {}
			} as any)

			const response = await store.deleteSink('sink-c1-01', [cluster1])

			expect(sinksApi.deleteSink).toHaveBeenCalledWith('sink-c1-01', [cluster1])
			expect(response?.success).toBe(true)
			expect(store.sinks).toHaveLength(1)
			expect(store.sinks[0].sink_id).toBe('sink-c2-02')
		})

		it('fetchSinkById retrieves sink details by id and cluster', async () => {
			const store = useSinksStore()

			const mockSink = {
				sink_id: 'sink-detail-01',
				name: 'Detail Sink',
				adm_status: 1,
				cluster_url: cluster1
			}

			vi.spyOn(sinksApi, 'getSinkById').mockResolvedValueOnce({
				status_code: 200,
				success: true,
				data: mockSink
			} as any)

			await store.fetchSinkById('sink-detail-01', [cluster1])

			expect(sinksApi.getSinkById).toHaveBeenCalledWith('sink-detail-01', [cluster1])
			expect(store.currentSink).toEqual(mockSink)
		})

		it('fetchSinks respects activeClusterURL from global appStore', async () => {
			const appStore = useAppStore()
			const store = useSinksStore()

			appStore.activeClusterURL = cluster1

			vi.spyOn(sinksApi, 'getSinks').mockResolvedValueOnce({
				status_code: 200,
				success: true,
				data: {
					sinks: [
						{
							sink_id: 'sink-c1-active',
							name: 'Active Cluster Sink',
							adm_status: 1,
							input_count: 0,
							labels: '{}',
							worker_count: 1,
							batch: { batch_size: 1000, batch_timeout: 1000 },
							channel: { type: 'memory' },
							config: { type: 'file', config: { path: '/tmp/active.log' } },
							last_updated: new Date().toISOString(),
							cluster_url: cluster1
						}
					],
					total: 1
				}
			} as any)

			await store.fetchSinks()

			expect(sinksApi.getSinks).toHaveBeenCalledWith(store.filter, undefined)
			expect(store.sinks).toHaveLength(1)
			expect(store.sinks[0].sink_id).toBe('sink-c1-active')
		})
	})
})

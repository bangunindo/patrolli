// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import '@@/shared/types/string.type'
import '@@/shared/types/array.type'
import { describe, it, expect, beforeEach, vi } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import processorApi from '@/app/api/processors.api'
import { useProcessorStore } from '@/stores/processor.store'
import { useAppStore } from '@/stores/app.store'
import { httpService } from '@/app/services/http.service'
import type { IProcessor, IProcessorForm } from '@@/shared/types/processor.type'

vi.mock('@/app/services/http.service', () => ({
	httpService: {
		get: vi.fn(),
		post: vi.fn(),
		put: vi.fn(),
		delete: vi.fn()
	}
}))

describe('Multicluster Configuration: Processors', () => {
	const cluster1 = '/patrolli-cluster-01'
	const cluster2 = '/patrolli-cluster-02'

	beforeEach(() => {
		setActivePinia(createPinia())
		vi.clearAllMocks()
	})

	describe('1. Processors API Multicluster Routing', () => {
		it('get routes GET to /api/processors on target cluster URL', async () => {
			const mockResponse = {
				status_code: 200,
				success: true,
				data: {
					processors: [
						{
							processor_id: 'proc-1',
							name: 'Processor 1',
							adm_status: 1,
							input_items: [],
							input_processors: [],
							config: { module: 'null' as const },
							last_updated: new Date().toISOString(),
							cluster_url: cluster1
						}
					],
					total: 1
				}
			}
			vi.mocked(httpService.get).mockResolvedValueOnce(mockResponse)

			const result = await processorApi.get({ limit: 10, offset: 0 }, [cluster1])

			expect(httpService.get).toHaveBeenCalledWith(
				'/api/processors',
				{ params: { limit: 10, offset: 0 } },
				[cluster1]
			)
			expect(result).toEqual(mockResponse)
		})

		it('create routes POST to /api/processors on target cluster URL', async () => {
			const payload: IProcessorForm = {
				name: 'Drop Debug Processor C1',
				adm_status: 1,
				input_items: [],
				input_processors: [],
				config: {
					module: 'drop_attributes',
					config: {
						keys: {
							record: ['debug_timestamp']
						}
					}
				}
			}
			const mockResponse = {
				status_code: 200,
				success: true,
				data: { processor_id: 'proc-c1-01', ...payload }
			}
			vi.mocked(httpService.post).mockResolvedValueOnce(mockResponse)

			const result = await processorApi.create(payload, [cluster1])

			expect(httpService.post).toHaveBeenCalledWith('/api/processors', payload, undefined, [
				cluster1
			])
			expect(result).toEqual(mockResponse)
		})

		it('update routes PUT to /api/processors/:id on target cluster URL', async () => {
			const payload: IProcessorForm = {
				name: 'Rename Processor C2',
				adm_status: 1,
				input_items: [],
				input_processors: [],
				config: {
					module: 'rename_attributes',
					config: {
						rename_map: {
							record: {
								cpu_usage: 'system_cpu_usage'
							}
						}
					}
				}
			}
			const mockResponse = {
				status_code: 200,
				success: true,
				data: { processor_id: 'proc-c2-01', ...payload }
			}
			vi.mocked(httpService.put).mockResolvedValueOnce(mockResponse)

			const result = await processorApi.update('proc-c2-01', payload, [cluster2])

			expect(httpService.put).toHaveBeenCalledWith(
				'/api/processors/proc-c2-01',
				payload,
				undefined,
				[cluster2]
			)
			expect(result).toEqual(mockResponse)
		})

		it('delete routes DELETE to /api/processors/:id on target cluster URL', async () => {
			const mockResponse = {
				status_code: 200,
				success: true,
				data: { processor_id: 'proc-c1-01' }
			}
			vi.mocked(httpService.delete).mockResolvedValueOnce(mockResponse)

			const result = await processorApi.delete('proc-c1-01', [cluster1])

			expect(httpService.delete).toHaveBeenCalledWith(
				'/api/processors/proc-c1-01',
				undefined,
				[cluster1]
			)
			expect(result).toEqual(mockResponse)
		})
	})

	describe('2. Processors Store Multicluster CRUD Operations', () => {
		it('createProcessor on Cluster 1 creates processor and refreshes list', async () => {
			const store = useProcessorStore()

			const payload: IProcessorForm = {
				name: 'Filter Metrics C1',
				adm_status: 1,
				input_items: [],
				input_processors: [],
				config: {
					module: 'null',
					config: undefined
				}
			}

			vi.spyOn(processorApi, 'create').mockResolvedValueOnce({
				status_code: 200,
				success: true,
				data: {
					processor_id: 'new-proc-01',
					...payload,
					last_updated: new Date().toISOString()
				} as any
			} as any)

			vi.spyOn(processorApi, 'get').mockResolvedValue({
				status_code: 200,
				success: true,
				data: {
					processors: [
						{
							processor_id: 'new-proc-01',
							name: 'Filter Metrics C1',
							adm_status: 1,
							input_items: [],
							input_processors: [],
							config: payload.config,
							last_updated: new Date().toISOString(),
							cluster_url: cluster1,
							cluster_name: 'patrolli-cluster-01'
						}
					],
					total: 1
				}
			} as any)

			const response = await store.createProcessor(payload, [cluster1])

			expect(processorApi.create).toHaveBeenCalledWith(payload, [cluster1])
			expect(processorApi.get).toHaveBeenCalled()
			expect(response?.success).toBe(true)
			expect(store.processors).toHaveLength(1)
			expect(store.processors[0].processor_id).toBe('new-proc-01')
		})

		it('updateProcessor on Cluster 2 updates processor and refreshes list', async () => {
			const store = useProcessorStore()

			const updatePayload: IProcessorForm = {
				name: 'Updated Filter C2',
				adm_status: 1,
				input_items: [],
				input_processors: [],
				config: {
					module: 'null',
					config: undefined
				}
			}

			vi.spyOn(processorApi, 'update').mockResolvedValueOnce({
				status_code: 200,
				success: true,
				data: {
					processor_id: 'proc-c2-01',
					...updatePayload,
					last_updated: new Date().toISOString()
				} as any
			} as any)

			vi.spyOn(processorApi, 'get').mockResolvedValue({
				status_code: 200,
				success: true,
				data: {
					processors: [
						{
							processor_id: 'proc-c2-01',
							name: 'Updated Filter C2',
							adm_status: 1,
							input_items: [],
							input_processors: [],
							config: updatePayload.config,
							last_updated: new Date().toISOString(),
							cluster_url: cluster2,
							cluster_name: 'patrolli-cluster-02'
						}
					],
					total: 1
				}
			} as any)

			const response = await store.updateProcessor('proc-c2-01', updatePayload, [cluster2])

			expect(processorApi.update).toHaveBeenCalledWith('proc-c2-01', updatePayload, [
				cluster2
			])
			expect(processorApi.get).toHaveBeenCalled()
			expect(response?.success).toBe(true)
			expect(store.processors).toHaveLength(1)
			expect(store.processors[0].name).toBe('Updated Filter C2')
		})

		it('deleteProcessor on Cluster 1 deletes processor and returns response', async () => {
			const store = useProcessorStore()

			vi.spyOn(processorApi, 'delete').mockResolvedValueOnce({
				status_code: 200,
				success: true,
				data: {}
			} as any)

			const response = await store.deleteProcessor('proc-c1-01', [cluster1])

			expect(processorApi.delete).toHaveBeenCalledWith('proc-c1-01', [cluster1])
			expect(response?.success).toBe(true)
		})

		it('fetchProcessorById retrieves target processor by id and cluster', async () => {
			const store = useProcessorStore()

			const mockProcessor: IProcessor = {
				processor_id: 'target-proc-id',
				name: 'Specific Processor',
				adm_status: 1,
				input_items: [],
				input_processors: [],
				config: { module: 'null', config: undefined },
				last_updated: new Date().toISOString(),
				cluster_url: cluster1
			}

			vi.spyOn(processorApi, 'get').mockResolvedValueOnce({
				status_code: 200,
				success: true,
				data: {
					processors: [mockProcessor],
					total: 1
				}
			} as any)

			const result = await store.fetchProcessorById('target-proc-id', [cluster1])

			expect(processorApi.get).toHaveBeenCalledWith({ name: undefined }, [cluster1])
			expect(result).toEqual(mockProcessor)
			expect(store.currentProcessor).toEqual(mockProcessor)
		})

		it('fetchProcessors respects activeClusterURL from global appStore', async () => {
			const appStore = useAppStore()
			const store = useProcessorStore()

			appStore.activeClusterURL = cluster1

			vi.spyOn(processorApi, 'get').mockResolvedValueOnce({
				status_code: 200,
				success: true,
				data: {
					processors: [
						{
							processor_id: 'proc-c1-active',
							name: 'Active Cluster Processor',
							adm_status: 1,
							input_items: [],
							input_processors: [],
							config: { module: 'null', config: undefined },
							last_updated: new Date().toISOString(),
							cluster_url: cluster1
						}
					],
					total: 1
				}
			} as any)

			await store.fetchProcessors()

			expect(processorApi.get).toHaveBeenCalledWith(store.filter, undefined)
			expect(store.processors).toHaveLength(1)
			expect(store.processors[0].processor_id).toBe('proc-c1-active')
		})
	})
})

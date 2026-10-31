// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import itemsApi from '@/app/api/items.api'
import templatesApi from '@/app/api/templates.api'
import { httpService } from '@/app/services/http.service'
import { useItemsStore } from '@/stores/items.store'
import { useTemplateItemsStore } from '@/stores/template-items.store'
import { FormMode } from '@@/shared/types/app.type'
import type { IItem, IItemForm, IItems } from '@@/shared/types/item.type'
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

describe('Multicluster CRUD for Template Items Feature', () => {
	const cluster1 = '/patrolli-cluster-01'
	const cluster2 = '/patrolli-cluster-02'
	const templateId1 = '11112222-3333-4444-5555-666677778888'

	const createMockItems = (partial: Partial<IItems> = {}): IItems => ({
		item_id: 'item-1',
		name: 'Mock Item',
		schedule: '60s',
		adm_status: 1,
		flags: 0,
		type: 'snmp',
		labels: '{}',
		last_updated: new Date().toISOString(),
		host_template_id: templateId1,
		outputs: [],
		root_item_id: 'item-1',
		...partial
	})

	const createMockItem = (partial: Partial<IItem> = {}): IItem => ({
		item_id: 'item-1',
		name: 'Mock Item Detail',
		schedule: '60s',
		adm_status: 1,
		flags: 0,
		timeout_ms: 5000,
		type: 'snmp',
		labels: '{}',
		last_updated: new Date().toISOString(),
		host_template_id: templateId1,
		outputs: [],
		root_item_id: 'item-1',
		config: {
			type: 'snmp',
			config: { oid: '1.3.6.1.4.1.2021.4.6.0' }
		},
		...partial
	})

	beforeEach(() => {
		setActivePinia(createPinia())
		vi.clearAllMocks()
	})

	describe('1. Template Items API Multicluster Routing', () => {
		it('createItemForTemplate routes POST to /api/items/template/:templateId on target cluster URL', async () => {
			const payload: IItemForm = {
				name: 'CPU Load Template Item',
				host_id: templateId1,
				schedule: '60s',
				adm_status: 1,
				flags: 0,
				timeout_ms: 5000,
				labels: '{"tier":"system"}',
				config: {
					type: 'snmp',
					config: {
						oid: '1.3.6.1.4.1.2021.10.1.3.1'
					}
				}
			}
			const mockResponse = {
				status_code: 200,
				success: true,
				data: createMockItem({ item_id: 'item-tmpl-01', ...payload })
			}
			vi.mocked(httpService.post).mockResolvedValueOnce(mockResponse)

			const result = await itemsApi.createItemForTemplate(templateId1, payload, [cluster1])

			expect(httpService.post).toHaveBeenCalledWith(
				`/api/items/template/${templateId1}`,
				payload,
				undefined,
				[cluster1]
			)
			expect(result).toEqual(mockResponse)
		})

		it('updateItemTemplate routes PUT to /api/items/:id/template on target cluster URL', async () => {
			const payload: IItemForm = {
				name: 'Updated CPU Load Template Item',
				host_id: templateId1,
				schedule: '120s',
				adm_status: 1,
				flags: 0,
				timeout_ms: 5000,
				labels: '{"tier":"system"}',
				config: {
					type: 'snmp',
					config: {
						oid: '1.3.6.1.4.1.2021.10.1.3.2'
					}
				}
			}
			const mockResponse = {
				status_code: 200,
				success: true,
				data: createMockItem({ item_id: 'item-tmpl-01', ...payload })
			}
			vi.mocked(httpService.put).mockResolvedValueOnce(mockResponse)

			const result = await itemsApi.updateItemTemplate('item-tmpl-01', payload, [cluster2])

			expect(httpService.put).toHaveBeenCalledWith(
				'/api/items/item-tmpl-01/template',
				payload,
				undefined,
				[cluster2]
			)
			expect(result).toEqual(mockResponse)
		})

		it('updateItemApliedTemplate routes PUT to /api/items/:id/applied-template on target cluster URL', async () => {
			const payload: IItemForm = {
				name: 'Applied Item Override',
				host_id: 'host-1',
				schedule: '30s',
				adm_status: 1,
				flags: 0,
				timeout_ms: 5000,
				config: {
					type: 'snmp',
					config: {
						oid: '1.3.6.1.4.1.2021.10.1.3.1'
					}
				}
			}
			const mockResponse = {
				status_code: 200,
				success: true,
				data: createMockItem({ item_id: 'item-applied-01', ...payload })
			}
			vi.mocked(httpService.put).mockResolvedValueOnce(mockResponse)

			const result = await itemsApi.updateItemApliedTemplate('item-applied-01', payload, [
				cluster1
			])

			expect(httpService.put).toHaveBeenCalledWith(
				'/api/items/item-applied-01/applied-template',
				payload,
				undefined,
				[cluster1]
			)
			expect(result).toEqual(mockResponse)
		})

		it('deleteItem routes DELETE to /api/items/:id on target cluster URL', async () => {
			const mockResponse = {
				status_code: 200,
				success: true,
				data: createMockItem({ item_id: 'item-tmpl-01' })
			}
			vi.mocked(httpService.delete).mockResolvedValueOnce(mockResponse)

			const result = await itemsApi.deleteItem('item-tmpl-01', [cluster1])

			expect(httpService.delete).toHaveBeenCalledWith('/api/items/item-tmpl-01', undefined, [
				cluster1
			])
			expect(result).toEqual(mockResponse)
		})
	})

	describe('2. Template Items Store Multicluster CRUD Operations', () => {
		it('setTemplateId sets templateId and filter.template_ids', () => {
			const store = useTemplateItemsStore()
			store.setTemplateId(templateId1)

			expect(store.templateId).toBe(templateId1)
			expect(store.filter.template_ids).toEqual([templateId1])
			expect(store.filter.exclusivity).toEqual(['template'])
		})

		it('fetchTemplate retrieves template metadata on target cluster', async () => {
			const store = useTemplateItemsStore()
			store.setTemplateId(templateId1)

			const mockTemplate = {
				template_id: templateId1,
				name: 'Linux Generic Base',
				labels: '{"os":"linux"}',
				cluster_url: cluster1
			}

			vi.spyOn(templatesApi, 'getTemplateById').mockResolvedValueOnce({
				status_code: 200,
				success: true,
				data: mockTemplate
			} as any)

			const result = await store.fetchTemplate([cluster1])

			expect(templatesApi.getTemplateById).toHaveBeenCalledWith(templateId1, [cluster1])
			expect(result).toEqual(mockTemplate)
			expect(store.currentTemplate).toEqual(mockTemplate)
		})

		it('fetchItems retrieves template items on target cluster', async () => {
			const store = useTemplateItemsStore()
			store.setTemplateId(templateId1)

			const mockItems: IItems[] = [
				createMockItems({
					item_id: 'item-1',
					name: 'Memory Free',
					cluster_url: cluster1
				})
			]

			vi.spyOn(itemsApi, 'getItems').mockResolvedValueOnce({
				status_code: 200,
				success: true,
				data: {
					items: mockItems,
					total: 1
				}
			} as any)

			await store.fetchItems([cluster1])

			expect(itemsApi.getItems).toHaveBeenCalledWith(store.filter, [cluster1])
			expect(store.items).toHaveLength(1)
			expect(store.items[0].item_id).toBe('item-1')
		})

		it('fetchItemById retrieves template item details on target cluster', async () => {
			const store = useTemplateItemsStore()

			const mockDetail: IItem = createMockItem({
				item_id: 'item-1',
				name: 'Memory Free',
				cluster_url: cluster1
			})

			vi.spyOn(itemsApi, 'getItemById').mockResolvedValueOnce({
				status_code: 200,
				success: true,
				data: mockDetail
			} as any)

			const result = await store.fetchItemById('item-1', [cluster1])

			expect(itemsApi.getItemById).toHaveBeenCalledWith('item-1', [cluster1])
			expect(result).toEqual(mockDetail)
			expect(store.currentItem).toEqual(mockDetail)
		})

		it('deleteItem on Cluster 1 deletes template item and removes from store', async () => {
			const store = useTemplateItemsStore()
			store.items = [
				createMockItems({
					item_id: 'item-1',
					name: 'Item 1',
					cluster_url: cluster1
				}),
				createMockItems({
					item_id: 'item-2',
					name: 'Item 2',
					cluster_url: cluster2
				})
			]

			vi.spyOn(itemsApi, 'deleteItem').mockResolvedValueOnce({
				status_code: 200,
				success: true,
				data: {}
			} as any)

			const response = await store.deleteItem('item-1', [cluster1])

			expect(itemsApi.deleteItem).toHaveBeenCalledWith('item-1', [cluster1])
			expect(response?.success).toBe(true)
			expect(store.items).toHaveLength(1)
			expect(store.items[0].item_id).toBe('item-2')
		})

		it('createItemForTemplate in itemsStore creates item on cluster and refreshes items', async () => {
			const itemsStore = useItemsStore()

			const payload: IItemForm = {
				name: 'Disk Free Item',
				host_id: templateId1,
				schedule: '60s',
				adm_status: 1,
				flags: 0,
				timeout_ms: 5000,
				config: {
					type: 'snmp',
					config: { oid: '1.3.6.1.4.1.2021.9.1.7.1' }
				}
			}

			const createdItem = createMockItem({
				item_id: 'item-disk-01',
				name: payload.name,
				schedule: payload.schedule,
				config: payload.config,
				cluster_url: cluster1
			})

			const createSpy = vi.spyOn(itemsApi, 'createItemForTemplate').mockResolvedValueOnce({
				status_code: 200,
				success: true,
				data: createdItem
			} as any)

			vi.spyOn(itemsApi, 'getItems').mockResolvedValue({
				status_code: 200,
				success: true,
				data: {
					items: [
						createMockItems({
							item_id: 'item-disk-01',
							name: payload.name,
							schedule: payload.schedule,
							cluster_url: cluster1
						})
					],
					total: 1
				}
			} as any)

			const res = await itemsStore.createItemForTemplate(templateId1, payload, [cluster1])

			expect(createSpy).toHaveBeenCalledWith(templateId1, payload, [cluster1])
			expect(itemsApi.getItems).toHaveBeenCalled()
			expect(res?.success).toBe(true)
			expect(itemsStore.items).toHaveLength(1)
		})

		it('toggleItemsModalForm and resetStore reset state correctly', () => {
			const store = useTemplateItemsStore()

			store.setTemplateId(templateId1)
			store.itemsModalShow = true
			store.itemsFormMode = FormMode.UPDATE
			store.items = [createMockItems({ item_id: 'item-1' })]

			store.toggleItemsModalForm(false)
			expect(store.itemsModalShow).toBe(false)

			store.resetStore()
			expect(store.templateId).toBe('')
			expect(store.currentTemplate).toBeNull()
			expect(store.items).toHaveLength(0)
			expect(store.currentItem).toBeNull()
			expect(store.filter.template_ids).toBeUndefined()
		})
	})
})

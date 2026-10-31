// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import templatesApi from '@/app/api/templates.api'
import { httpService } from '@/app/services/http.service'
import { useAppStore } from '@/stores/app.store'
import { useTemplatesStore } from '@/stores/templates.store'
import type { ITemplate, ITemplateForm } from '@@/shared/types/template.type'
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

describe('Multicluster CRUD for Templates Feature', () => {
	const cluster1 = '/patrolli-cluster-01'
	const cluster2 = '/patrolli-cluster-02'

	beforeEach(() => {
		setActivePinia(createPinia())
		vi.clearAllMocks()
	})

	describe('1. Templates API Multicluster Routing', () => {
		it('getTemplates routes GET to /api/templates on target cluster URL', async () => {
			const mockResponse = {
				status_code: 200,
				success: true,
				data: {
					templates: [
						{
							template_id: 'tmpl-1',
							name: 'Template 1',
							labels: '{}',
							item_count: 5,
							cluster_url: cluster1
						}
					],
					total: 1
				}
			}
			vi.mocked(httpService.get).mockResolvedValueOnce(mockResponse)

			const result = await templatesApi.getTemplates({ limit: 10, offset: 0 }, [cluster1])

			expect(httpService.get).toHaveBeenCalledWith(
				'/api/templates',
				{ params: { limit: 10, offset: 0 } },
				[cluster1]
			)
			expect(result).toEqual(mockResponse)
		})

		it('createTemplate routes POST to /api/templates on target cluster URL', async () => {
			const payload: ITemplateForm = {
				name: 'Linux Server Base C1',
				labels: '{"os":"linux"}'
			}
			const mockResponse = {
				status_code: 200,
				success: true,
				data: { template_id: 'tmpl-c1-01', ...payload }
			}
			vi.mocked(httpService.post).mockResolvedValueOnce(mockResponse)

			const result = await templatesApi.createTemplate(payload, [cluster1])

			expect(httpService.post).toHaveBeenCalledWith('/api/templates', payload, undefined, [
				cluster1
			])
			expect(result).toEqual(mockResponse)
		})

		it('getTemplateById routes GET to /api/templates/:id on target cluster URL', async () => {
			const mockResponse = {
				status_code: 200,
				success: true,
				data: {
					template_id: 'tmpl-c1-01',
					name: 'Linux Server Base C1',
					cluster_url: cluster1
				}
			}
			vi.mocked(httpService.get).mockResolvedValueOnce(mockResponse)

			const result = await templatesApi.getTemplateById('tmpl-c1-01', [cluster1])

			expect(httpService.get).toHaveBeenCalledWith('/api/templates/tmpl-c1-01', undefined, [
				cluster1
			])
			expect(result).toEqual(mockResponse)
		})

		it('updateTemplate routes PUT to /api/templates/:id on target cluster URL', async () => {
			const payload: ITemplateForm = {
				name: 'Updated Linux Server C2',
				labels: '{"os":"linux","version":"2.0"}'
			}
			const mockResponse = {
				status_code: 200,
				success: true,
				data: { template_id: 'tmpl-c2-01', ...payload }
			}
			vi.mocked(httpService.put).mockResolvedValueOnce(mockResponse)

			const result = await templatesApi.updateTemplate('tmpl-c2-01', payload, [cluster2])

			expect(httpService.put).toHaveBeenCalledWith(
				'/api/templates/tmpl-c2-01',
				payload,
				undefined,
				[cluster2]
			)
			expect(result).toEqual(mockResponse)
		})

		it('deleteTemplate routes DELETE to /api/templates/:id on target cluster URL', async () => {
			const mockResponse = {
				status_code: 200,
				success: true,
				data: { template_id: 'tmpl-c1-01' }
			}
			vi.mocked(httpService.delete).mockResolvedValueOnce(mockResponse)

			const result = await templatesApi.deleteTemplate('tmpl-c1-01', [cluster1])

			expect(httpService.delete).toHaveBeenCalledWith(
				'/api/templates/tmpl-c1-01',
				undefined,
				[cluster1]
			)
			expect(result).toEqual(mockResponse)
		})

		it('linkTemplate and unlinkTemplate route PUT on target cluster URL', async () => {
			vi.mocked(httpService.put).mockResolvedValue({ status_code: 200, success: true })

			await templatesApi.linkTemplate('tmpl-parent', { link_to_template_id: 'tmpl-child' }, [
				cluster1
			])
			expect(httpService.put).toHaveBeenCalledWith(
				'/api/templates/link/tmpl-parent',
				{ link_to_template_id: 'tmpl-child' },
				undefined,
				[cluster1]
			)

			await templatesApi.unlinkTemplate(
				'tmpl-parent',
				{ unlink_from_template_id: 'tmpl-child' },
				[cluster2]
			)
			expect(httpService.put).toHaveBeenCalledWith(
				'/api/templates/unlink/tmpl-parent',
				{ unlink_from_template_id: 'tmpl-child' },
				undefined,
				[cluster2]
			)
		})

		it('applyTemplate and unapplyTemplate route POST on target cluster URL', async () => {
			vi.mocked(httpService.post).mockResolvedValue({ status_code: 200, success: true })

			await templatesApi.applyTemplate({ host_ids: ['host-1'], template_ids: ['tmpl-1'] }, [
				cluster1
			])
			expect(httpService.post).toHaveBeenCalledWith(
				'/api/templates/apply',
				{ host_ids: ['host-1'], template_ids: ['tmpl-1'] },
				undefined,
				[cluster1]
			)

			await templatesApi.unapplyTemplate(
				{
					unapply_list: [{ host_id: 'host-1', template_id: 'tmpl-1', keep_items: true }]
				},
				[cluster2]
			)
			expect(httpService.post).toHaveBeenCalledWith(
				'/api/templates/unapply',
				{
					unapply_list: [{ host_id: 'host-1', template_id: 'tmpl-1', keep_items: true }]
				},
				undefined,
				[cluster2]
			)
		})
	})

	describe('2. Templates Store Multicluster CRUD Operations', () => {
		it('createTemplate on Cluster 1 creates template and refreshes list', async () => {
			const store = useTemplatesStore()

			const payload: ITemplateForm = {
				name: 'SNMP Generic Router C1',
				labels: '{"env":"prod"}'
			}

			vi.spyOn(templatesApi, 'createTemplate').mockResolvedValueOnce({
				status_code: 200,
				success: true,
				data: {
					template_id: 'new-tmpl-01',
					...payload,
					item_count: 0
				}
			} as any)

			vi.spyOn(templatesApi, 'getTemplates').mockResolvedValue({
				status_code: 200,
				success: true,
				data: {
					templates: [
						{
							template_id: 'new-tmpl-01',
							name: 'SNMP Generic Router C1',
							labels: payload.labels,
							item_count: 0,
							cluster_url: cluster1,
							cluster_name: 'patrolli-cluster-01'
						}
					],
					total: 1
				}
			} as any)

			const response = await store.createTemplate(payload, [cluster1])

			expect(templatesApi.createTemplate).toHaveBeenCalledWith(payload, [cluster1])
			expect(templatesApi.getTemplates).toHaveBeenCalled()
			expect(response?.success).toBe(true)
			expect(store.templates).toHaveLength(1)
			expect(store.templates[0].template_id).toBe('new-tmpl-01')
		})

		it('updateTemplate on Cluster 2 updates template and refreshes list', async () => {
			const store = useTemplatesStore()

			const updatePayload: ITemplateForm = {
				name: 'Updated Template C2',
				labels: '{"env":"staging"}'
			}

			vi.spyOn(templatesApi, 'updateTemplate').mockResolvedValueOnce({
				status_code: 200,
				success: true,
				data: {
					template_id: 'tmpl-c2-01',
					...updatePayload
				}
			} as any)

			vi.spyOn(templatesApi, 'getTemplates').mockResolvedValue({
				status_code: 200,
				success: true,
				data: {
					templates: [
						{
							template_id: 'tmpl-c2-01',
							name: 'Updated Template C2',
							labels: updatePayload.labels,
							cluster_url: cluster2,
							cluster_name: 'patrolli-cluster-02'
						}
					],
					total: 1
				}
			} as any)

			const response = await store.updateTemplate('tmpl-c2-01', updatePayload, [cluster2])

			expect(templatesApi.updateTemplate).toHaveBeenCalledWith('tmpl-c2-01', updatePayload, [
				cluster2
			])
			expect(templatesApi.getTemplates).toHaveBeenCalled()
			expect(response?.success).toBe(true)
			expect(store.templates).toHaveLength(1)
			expect(store.templates[0].name).toBe('Updated Template C2')
		})

		it('deleteTemplate on Cluster 1 deletes template and removes from store', async () => {
			const store = useTemplatesStore()
			store.templates = [
				{
					template_id: 'tmpl-c1-01',
					name: 'Template 1',
					labels: '{}',
					cluster_url: cluster1
				},
				{
					template_id: 'tmpl-c2-02',
					name: 'Template 2',
					labels: '{}',
					cluster_url: cluster2
				}
			]

			vi.spyOn(templatesApi, 'deleteTemplate').mockResolvedValueOnce({
				status_code: 200,
				success: true,
				data: {}
			} as any)

			const response = await store.deleteTemplate('tmpl-c1-01', [cluster1])

			expect(templatesApi.deleteTemplate).toHaveBeenCalledWith('tmpl-c1-01', [cluster1])
			expect(response?.success).toBe(true)
			expect(store.templates).toHaveLength(1)
			expect(store.templates[0].template_id).toBe('tmpl-c2-02')
		})

		it('fetchTemplatesDetail retrieves template details by id and cluster', async () => {
			const store = useTemplatesStore()

			const mockTemplate: ITemplate = {
				template_id: 'tmpl-detail-01',
				name: 'Detail Template',
				labels: '{"tier":"frontend"}',
				cluster_url: cluster1
			}

			vi.spyOn(templatesApi, 'getTemplateById').mockResolvedValueOnce({
				status_code: 200,
				success: true,
				data: mockTemplate
			} as any)

			const result = await store.fetchTemplatesDetail('tmpl-detail-01', [cluster1])

			expect(templatesApi.getTemplateById).toHaveBeenCalledWith('tmpl-detail-01', [cluster1])
			expect(result).toEqual(mockTemplate)
		})

		it('linkTemplatesFor and unlinkTemplatesFor call api with cluster target', async () => {
			const store = useTemplatesStore()

			const linkSpy = vi
				.spyOn(templatesApi, 'linkTemplate')
				.mockResolvedValue({ status_code: 200, success: true } as any)
			const unlinkSpy = vi
				.spyOn(templatesApi, 'unlinkTemplate')
				.mockResolvedValue({ status_code: 200, success: true } as any)

			await store.linkTemplatesFor(
				'tmpl-parent-01',
				['tmpl-child-01', 'tmpl-child-02'],
				[cluster1]
			)
			expect(linkSpy).toHaveBeenCalledTimes(2)
			expect(linkSpy).toHaveBeenCalledWith(
				'tmpl-parent-01',
				{ link_to_template_id: 'tmpl-child-01' },
				[cluster1]
			)

			await store.unlinkTemplatesFor(
				'tmpl-parent-01',
				[{ unlink_from_template_id: 'tmpl-child-01', keep_items: true }],
				[cluster1]
			)
			expect(unlinkSpy).toHaveBeenCalledWith(
				'tmpl-parent-01',
				{ unlink_from_template_id: 'tmpl-child-01', keep_items: true },
				[cluster1]
			)
		})

		it('fetchTemplates respects activeClusterURL from global appStore', async () => {
			const appStore = useAppStore()
			const store = useTemplatesStore()

			appStore.activeClusterURL = cluster1

			vi.spyOn(templatesApi, 'getTemplates').mockResolvedValueOnce({
				status_code: 200,
				success: true,
				data: {
					templates: [
						{
							template_id: 'tmpl-c1-active',
							name: 'Active Cluster Template',
							labels: '{}',
							cluster_url: cluster1
						}
					],
					total: 1
				}
			} as any)

			await store.fetchTemplates()

			expect(templatesApi.getTemplates).toHaveBeenCalledWith(store.filter, undefined)
			expect(store.templates).toHaveLength(1)
			expect(store.templates[0].template_id).toBe('tmpl-c1-active')
		})
	})
})

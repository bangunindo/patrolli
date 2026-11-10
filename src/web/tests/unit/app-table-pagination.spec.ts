// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { reactive, nextTick } from 'vue'
import AppTable from '@/components/app/AppTable.vue'

describe('AppTable Pagination State Management', () => {
	const columns = [
		{ key: 'id', title: 'ID' },
		{ key: 'name', title: 'Name' }
	]

	const sampleData = Array.from({ length: 25 }, (_, i) => ({
		id: i + 1,
		name: `Item ${i + 1}`
	}))

	it('computes currentPage and pagination info correctly', () => {
		const pagination = {
			rows: 10,
			currentPage: 2,
			totalRecords: 25
		}

		const wrapper = mount(AppTable, {
			props: {
				columns,
				data: sampleData.slice(10, 20),
				pagination
			},
			global: {
				stubs: {
					UiIcon: true,
					UiPagination: true,
					UiPaginationList: true,
					UiPaginationFirst: true,
					UiPaginationPrev: true,
					UiPaginationNext: true,
					UiPaginationLast: true,
					UiPaginationListItem: true,
					UiPaginationEllipsis: true,
					UiButton: true,
					UiCheckbox: true
				}
			}
		})

		const text = wrapper.find('.pagination-info').text()
		expect(text).toContain('11-20')
		expect(text).toContain('25')
	})

	it('derives currentPage from offset and rows if currentPage is undefined', () => {
		const pagination = {
			limit: 10,
			offset: 20,
			total: 35
		}

		const wrapper = mount(AppTable, {
			props: {
				columns,
				data: sampleData.slice(20, 25),
				pagination
			},
			global: {
				stubs: {
					UiIcon: true,
					UiPagination: true,
					UiPaginationList: true,
					UiPaginationFirst: true,
					UiPaginationPrev: true,
					UiPaginationNext: true,
					UiPaginationLast: true,
					UiPaginationListItem: true,
					UiPaginationEllipsis: true,
					UiButton: true,
					UiCheckbox: true
				}
			}
		})

		const text = wrapper.find('.pagination-info').text()
		expect(text).toContain('21-30')
		expect(text).toContain('35')
	})

	it('updates internal currentPage and emits update:page', async () => {
		const pagination = reactive({
			rows: 10,
			currentPage: 1,
			totalRecords: 25
		})

		const wrapper = mount(AppTable, {
			props: {
				columns,
				data: sampleData.slice(0, 10),
				pagination
			},
			global: {
				stubs: {
					UiIcon: true,
					UiPagination: {
						props: ['page', 'total', 'itemsPerPage'],
						template: `
							<div class="ui-pagination-stub">
								<button class="goto-page-2" @click="$emit('update:page', 2)">Page 2</button>
							</div>
						`
					},
					UiPaginationList: true,
					UiPaginationFirst: true,
					UiPaginationPrev: true,
					UiPaginationNext: true,
					UiPaginationLast: true,
					UiPaginationListItem: true,
					UiPaginationEllipsis: true,
					UiButton: true,
					UiCheckbox: true
				}
			}
		})

		await wrapper.find('.goto-page-2').trigger('click')
		await flushPromises()

		// Emitted event payload verification
		expect(wrapper.emitted('update:page')).toBeTruthy()
		expect(wrapper.emitted('update:page')![0][0]).toEqual({
			limit: 10,
			offset: 10,
			page: 2
		})

		// Internal pagination text reflects page 2
		expect(wrapper.find('.pagination-info').text()).toContain('11-20')
	})

	it('auto-clamps currentPage when totalRecords drops below current page offset', async () => {
		const pagination = reactive({
			rows: 10,
			currentPage: 3,
			totalRecords: 30
		})

		const wrapper = mount(AppTable, {
			props: {
				columns,
				data: sampleData.slice(20, 25),
				pagination
			},
			global: {
				stubs: {
					UiIcon: true,
					UiPagination: true,
					UiPaginationList: true,
					UiPaginationFirst: true,
					UiPaginationPrev: true,
					UiPaginationNext: true,
					UiPaginationLast: true,
					UiPaginationListItem: true,
					UiPaginationEllipsis: true,
					UiButton: true,
					UiCheckbox: true
				}
			}
		})

		// Simulate deletion: totalRecords decreases from 30 to 15 (maxPage becomes 2)
		pagination.totalRecords = 15
		await nextTick()
		await flushPromises()

		expect(wrapper.emitted('update:page')).toBeTruthy()
		const lastEmission = wrapper.emitted('update:page')!.slice(-1)[0][0]
		expect(lastEmission).toEqual({
			limit: 10,
			offset: 10,
			page: 2
		})
		expect(wrapper.find('.pagination-info').text()).toContain('11-15')
	})
})

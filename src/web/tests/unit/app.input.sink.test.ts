// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import '@@/shared/types/string.type'
import '@@/shared/types/array.type'
import { describe, it, expect, beforeEach, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import AppInputSink from '@/components/app/input/AppInputSink.vue'
import sinksApi from '@/app/api/sinks.api'

describe('AppInputSink', () => {
	beforeEach(() => {
		vi.clearAllMocks()
	})

	const globalStubs = {
		AppInputSelectTable: {
			props: [
				'data',
				'loading',
				'pagination',
				'multiple',
				'valueKey',
				'checkboxKey',
				'headerTable',
				'modelValue'
			],
			emits: ['update:modelValue', 'search', 'infinite'],
			mounted() {
				;(this as any).$emit('infinite', { page: 1, offset: 0 })
			},
			template: `
				<div class="app-input-select-table-stub" :data-count="data.length" :data-multiple="multiple">
					<div v-for="item in data" :key="item.sink_id" class="sink-row">
						{{ item.name }} - {{ item.sink_id }}
					</div>
					<button class="select-first-btn" @click="$emit('update:modelValue', data[0]?.sink_id)">
						Select First
					</button>
				</div>
			`
		}
	}

	it('fetches sinks and passes them to AppInputSelectTable', async () => {
		const mockSinks = [
			{ sink_id: 'sink-1', name: 'Kafka Sink 1', adm_status: 1, type: 'kafka' },
			{ sink_id: 'sink-2', name: 'Clickhouse Sink 2', adm_status: 1, type: 'clickhouse' }
		]

		const getSinksSpy = vi.spyOn(sinksApi, 'getSinks').mockResolvedValue({
			success: true,
			data: {
				sinks: mockSinks,
				total: 2
			}
		} as any)

		const wrapper = mount(AppInputSink, {
			props: {
				multiple: false
			},
			global: {
				stubs: globalStubs
			}
		})

		await wrapper.vm.$nextTick()
		await new Promise((resolve) => setTimeout(resolve, 50))

		expect(getSinksSpy).toHaveBeenCalled()
		const stub = wrapper.find('.app-input-select-table-stub')
		expect(stub.exists()).toBe(true)
		expect(stub.attributes('data-multiple')).toBe('false')
		expect(wrapper.text()).toContain('Kafka Sink 1')
		expect(wrapper.text()).toContain('Clickhouse Sink 2')
	})

	it('constrains querying and display when sinkIds prop is provided', async () => {
		const mockSinks = [
			{ sink_id: 'sink-2', name: 'Clickhouse Sink 2', adm_status: 1, type: 'clickhouse' }
		]

		const getSinksSpy = vi.spyOn(sinksApi, 'getSinks').mockResolvedValue({
			success: true,
			data: {
				sinks: mockSinks,
				total: 1
			}
		} as any)

		const wrapper = mount(AppInputSink, {
			props: {
				sinkIds: ['sink-2'],
				multiple: false
			},
			global: {
				stubs: globalStubs
			}
		})

		await wrapper.vm.$nextTick()
		await new Promise((resolve) => setTimeout(resolve, 50))

		expect(getSinksSpy).toHaveBeenCalledWith(
			expect.objectContaining({
				sink_ids: ['sink-2']
			}),
			undefined
		)
		expect(wrapper.text()).toContain('Clickhouse Sink 2')
	})

	it('handles empty sinkIds prop without calling getSinks', async () => {
		const getSinksSpy = vi.spyOn(sinksApi, 'getSinks').mockResolvedValue({
			success: true,
			data: { sinks: [], total: 0 }
		} as any)

		const wrapper = mount(AppInputSink, {
			props: {
				sinkIds: [],
				multiple: false
			},
			global: {
				stubs: globalStubs
			}
		})

		await wrapper.vm.$nextTick()
		await new Promise((resolve) => setTimeout(resolve, 50))

		expect(getSinksSpy).not.toHaveBeenCalled()
		const stub = wrapper.find('.app-input-select-table-stub')
		expect(stub.attributes('data-count')).toBe('0')
	})
})

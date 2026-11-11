// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import '@@/shared/types/string.type'
import '@@/shared/types/array.type'
import { describe, it, expect, beforeEach, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import NodeConfigDrawer from '@/components/pipeline/builder/NodeConfigDrawer.vue'
import { usePipelineBuildStore } from '@/stores/pipeline.builder.store'
import sinksApi from '@/app/api/sinks.api'

describe('NodeConfigDrawer', () => {
	beforeEach(() => {
		setActivePinia(createPinia())
		vi.clearAllMocks()
	})

	const globalStubs = {
		AppInputText: {
			props: ['readonly', 'disabled', 'modelValue', 'placeholder'],
			emits: ['update:modelValue'],
			template:
				'<div class="app-input-text-stub"><input class="inner-input" :readonly="readonly" :disabled="disabled" :placeholder="placeholder" :value="modelValue" @input="$emit(\'update:modelValue\', $event.target.value)" /></div>'
		},
		AppInputSink: {
			props: ['modelValue', 'sinkIds', 'multiple', 'clusterUrl'],
			emits: ['update:modelValue'],
			template: `
				<div class="app-input-sink-stub" :data-sink-ids="JSON.stringify(sinkIds)" :data-multiple="multiple">
					<button class="select-sink-btn" @click="$emit('update:modelValue', 'sink-1')">Select Sink 1</button>
				</div>
			`
		}
	}

	it('renders node name as read only and does not render node type field for source node', () => {
		const store = usePipelineBuildStore()
		store.selectedNode = {
			id: 'source-1',
			node_id: 'source-1',
			type: 'source',
			label: 'Kafka Source',
			position: { x: 100, y: 100 },
			data: {
				id: 'source-1',
				name: 'Kafka Source',
				type: 'source'
			}
		} as any

		const wrapper = mount(NodeConfigDrawer, {
			global: {
				stubs: globalStubs
			}
		})

		const nameInput = wrapper.find('.app-input-text-stub .inner-input')
		expect(nameInput.exists()).toBe(true)
		expect(nameInput.attributes('readonly')).toBeDefined()
		expect((nameInput.element as HTMLInputElement).value).toBe('Kafka Source')
		expect(wrapper.text()).not.toContain('Node Type')
	})

	it('renders empty state warning when pipeline has no registered sinks', async () => {
		const store = usePipelineBuildStore()
		store.pipelineDetail = {
			pipeline_id: 'pipe-1',
			sink_ids: []
		} as any

		store.selectedNode = {
			id: 'sink-node-1',
			node_id: 'sink-node-1',
			type: 'sink',
			label: 'Sink Node',
			data: {
				id: 'sink-node-1',
				name: 'Sink Node'
			}
		} as any

		const wrapper = mount(NodeConfigDrawer, {
			global: {
				stubs: globalStubs
			}
		})

		expect(wrapper.find('[role="alert"]').text()).toContain(
			'No sinks registered in this pipeline'
		)
		expect(wrapper.find('.app-input-sink-stub').exists()).toBe(false)

		const selectSinksBtn = wrapper
			.findAll('button')
			.find((btn) => btn.text().includes('Select Sinks'))
		expect(selectSinksBtn).toBeDefined()

		const openSinkSpy = vi.spyOn(store, 'openSinkSelectorModal').mockResolvedValue()
		await selectSinksBtn!.trigger('click')
		expect(openSinkSpy).toHaveBeenCalledTimes(1)
	})

	it('renders AppInputSink constrained to pipeline registered sinks when sink node is selected', async () => {
		const store = usePipelineBuildStore()
		store.pipelineDetail = {
			pipeline_id: 'pipe-1',
			sink_ids: ['sink-1', 'sink-2']
		} as any

		store.selectedNode = {
			id: 'sink-node-1',
			node_id: 'sink-node-1',
			type: 'sink',
			label: 'Sink Node',
			data: {
				id: 'sink-node-1',
				name: 'Sink Node'
			}
		} as any

		const wrapper = mount(NodeConfigDrawer, {
			global: {
				stubs: globalStubs
			}
		})

		const sinkInput = wrapper.find('.app-input-sink-stub')
		expect(sinkInput.exists()).toBe(true)
		expect(sinkInput.attributes('data-sink-ids')).toBe(JSON.stringify(['sink-1', 'sink-2']))
		expect(sinkInput.attributes('data-multiple')).toBe('false')
	})

	it('renders Manage Sinks shortcut button when pipeline has registered sinks and opens sink selector modal', async () => {
		const store = usePipelineBuildStore()
		const openSinkModalSpy = vi.spyOn(store, 'openSinkSelectorModal').mockResolvedValue()
		store.pipelineDetail = {
			pipeline_id: 'pipe-1',
			sink_ids: ['sink-1']
		} as any
		store.selectedNode = {
			id: 'sink-node-1',
			node_id: 'sink-node-1',
			type: 'sink',
			label: 'Sink Node',
			data: {
				id: 'sink-node-1',
				name: 'Sink Node'
			}
		} as any

		const wrapper = mount(NodeConfigDrawer, {
			global: {
				stubs: globalStubs
			}
		})

		const manageBtn = wrapper.findAll('button').find((b) => b.text().includes('Manage Sinks'))
		expect(manageBtn).toBeDefined()

		await manageBtn!.trigger('click')
		expect(openSinkModalSpy).toHaveBeenCalledTimes(1)
	})

	it('fetches sink details and renders full details matching ModalDetail when a sink is selected', async () => {
		const mockSinkDetail = {
			sink_id: 'sink-1',
			name: 'Audit Log Kafka Sink',
			adm_status: 1,
			worker_count: 4,
			input_count: 12,
			last_updated: '2026-10-22T08:00:00Z',
			config: {
				type: 'kafka',
				config: {
					topic: 'audit-logs',
					brokers: ['kafka.internal:9092']
				}
			},
			batch: {
				batch_size: 500,
				batch_timeout: 200
			},
			channel: {
				type: 'memory',
				capacity: 50000
			},
			labels: '{"environment":"production","team":"security"}'
		}

		const getSinkByIdSpy = vi.spyOn(sinksApi, 'getSinkById').mockResolvedValue({
			success: true,
			data: mockSinkDetail
		} as any)

		const store = usePipelineBuildStore()
		store.pipelineDetail = {
			pipeline_id: 'pipe-1',
			sink_ids: ['sink-1']
		} as any

		store.selectedNode = {
			id: 'sink-node-1',
			node_id: 'sink-node-1',
			type: 'sink',
			label: 'Sink Node',
			data: {
				id: 'sink-node-1',
				name: 'Sink Node',
				sinkId: 'sink-1'
			}
		} as any

		const wrapper = mount(NodeConfigDrawer, {
			global: {
				stubs: globalStubs
			}
		})

		await wrapper.vm.$nextTick()
		await new Promise((resolve) => setTimeout(resolve, 10))

		expect(getSinkByIdSpy).toHaveBeenCalledWith('sink-1', undefined)

		expect(wrapper.text()).toContain('Sink Target Details')
		expect(wrapper.text()).toContain('Audit Log Kafka Sink')
		expect(wrapper.text()).toContain('Active')
		expect(wrapper.text()).toContain('Worker Count')
		expect(wrapper.text()).toContain('4')
		expect(wrapper.text()).toContain('Input Count')
		expect(wrapper.text()).toContain('12')
		expect(wrapper.text()).toContain('audit-logs')
		expect(wrapper.text()).toContain('environment : production')
		expect(wrapper.text()).toContain('team : security')
	})

	it('retries getSinkById after a failed fetch when sink node is re-selected', async () => {
		const store = usePipelineBuildStore()
		store.pipelineDetail = {
			pipeline_id: 'pipe-1',
			sink_ids: ['sink-1']
		} as any
		store.availableSinks = [{ sink_id: 'sink-1', name: 'Fallback Sink', type: 'kafka' }]

		const getSinkByIdSpy = vi
			.spyOn(sinksApi, 'getSinkById')
			.mockRejectedValueOnce(new Error('Temporary network error'))
			.mockResolvedValueOnce({
				success: true,
				data: {
					sink_id: 'sink-1',
					name: 'Recovered Kafka Sink',
					adm_status: 1,
					worker_count: 2,
					input_count: 5,
					last_updated: '2026-11-11T00:00:00Z'
				}
			} as any)

		store.selectedNode = {
			id: 'sink-node-1',
			node_id: 'sink-node-1',
			type: 'sink',
			label: 'Sink Node',
			data: { id: 'sink-node-1', name: 'Sink Node', sinkId: 'sink-1' }
		} as any

		const wrapper = mount(NodeConfigDrawer, {
			global: {
				stubs: globalStubs
			}
		})

		await wrapper.vm.$nextTick()
		await new Promise((resolve) => setTimeout(resolve, 10))
		expect(getSinkByIdSpy).toHaveBeenCalledTimes(1)
		expect(wrapper.text()).toContain('Fallback Sink')

		// Re-selecting the same sink node should retry getSinkById because the first call failed
		store.selectedNode = { ...store.selectedNode }
		await wrapper.vm.$nextTick()
		await new Promise((resolve) => setTimeout(resolve, 10))

		expect(getSinkByIdSpy).toHaveBeenCalledTimes(2)
		expect(wrapper.text()).toContain('Recovered Kafka Sink')
	})

	it('trims leading and trailing whitespace when filtering associated items on a source node and resolves host names from store.hostById', async () => {
		const store = usePipelineBuildStore()
		store.pipelineDetail = {
			pipeline_id: 'pipe-1',
			host_ids: ['host-1', 'host-unresolved']
		} as any
		store.availableHosts = [{ host_id: 'host-1', name: 'Core Switch 01' }] as any
		store.pipelineItems = [
			{ item_id: 'item-1', host_id: 'host-1', name: 'CPU Load', type: 'snmp' },
			{ item_id: 'item-2', host_id: 'host-unresolved', name: 'Disk Usage', type: 'agent' }
		] as any
		store.selectedNode = {
			id: 'source-1',
			node_id: 'source-1',
			type: 'source',
			label: 'Source',
			position: { x: 80, y: 160 },
			data: {
				id: 'source-1',
				name: 'Source',
				type: 'source',
				items: ['item-1', 'item-2'],
				item_count: 2
			}
		} as any

		const wrapper = mount(NodeConfigDrawer, {
			global: {
				stubs: globalStubs
			}
		})

		expect(wrapper.text()).toContain('Selected Hosts (2)')
		expect(wrapper.text()).toContain('Core Switch 01')
		expect(wrapper.text()).toContain('host-unresolved')

		const searchInput = wrapper.find('input[placeholder="Search items by name..."]')
		expect(searchInput.exists()).toBe(true)
		await searchInput.setValue('  CPU  ')

		expect(wrapper.text()).toContain('CPU Load')
		expect(wrapper.text()).not.toContain('Disk Usage')

		await searchInput.setValue('  core switch  ')
		expect(wrapper.text()).toContain('CPU Load')
		expect(wrapper.text()).not.toContain('Disk Usage')
	})
})

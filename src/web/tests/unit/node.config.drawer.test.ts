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
		'b-input': {
			props: ['readonly', 'disabled', 'modelValue'],
			template:
				'<input class="b-input-stub" :readonly="readonly" :disabled="disabled" :value="modelValue" />'
		},
		'b-icon': {
			template: '<i class="b-icon-stub"></i>'
		},
		'b-heading': {
			template: '<h6 class="b-heading-stub"><slot /></h6>'
		},
		'b-spinner': {
			template: '<div class="b-spinner-stub"></div>'
		},
		'b-flex': {
			template: '<div class="b-flex-stub"><slot /></div>'
		},
		'b-tag': {
			template: '<span class="b-tag-stub"><slot /></span>'
		},
		AppInputText: {
			props: ['readonly', 'disabled', 'modelValue', 'placeholder'],
			template:
				'<div class="app-input-text-stub"><input class="inner-input" :readonly="readonly" :disabled="disabled" :value="modelValue" /></div>'
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

	it('renders empty state warning when pipeline has no registered sinks', () => {
		const store = usePipelineBuildStore()
		store.pipelineDetail = {
			pipeline_management_id: 'pipe-1',
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

		expect(wrapper.text()).toContain('No sinks registered in this pipeline')
		expect(wrapper.find('.app-input-sink-stub').exists()).toBe(false)
	})

	it('renders AppInputSink constrained to pipeline registered sinks when sink node is selected', async () => {
		const store = usePipelineBuildStore()
		store.pipelineDetail = {
			pipeline_management_id: 'pipe-1',
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
			pipeline_management_id: 'pipe-1',
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
})

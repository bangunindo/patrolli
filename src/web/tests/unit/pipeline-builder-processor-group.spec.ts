// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import '@@/shared/types/string.type'
import '@@/shared/types/array.type'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import { setActivePinia, createPinia } from 'pinia'
import { usePipelineBuildStore } from '@/stores/pipeline.builder.store'
import { useProcessorStore } from '@/stores/processor.store'
import { FormMode } from '@@/shared/types/app.type'
import pipelineManagementApi from '@/app/api/pipeline.management.api'
import processorsApi from '@/app/api/processors.api'
import EdgeItemsModal from '@/components/pipeline/builder/modals/EdgeItemsModal.vue'
import ProcessorGroupDrawer from '@/components/pipeline/builder/ProcessorGroupDrawer.vue'
import Form from '@/components/configuration/processor/Form.vue'
import BuilderProcessorNode from '@/components/pipeline/builder/nodes/BuilderProcessorNode.vue'
import BuilderSinkNode from '@/components/pipeline/builder/nodes/BuilderSinkNode.vue'
import BuilderCanvas from '@/components/pipeline/builder/BuilderCanvas.vue'
import PreviewDataModal from '@/components/pipeline/builder/modals/PreviewDataModal.vue'

describe('Pipeline Builder - Processor Group Feature', () => {
	beforeEach(() => {
		setActivePinia(createPinia())
		vi.clearAllMocks()

		vi.spyOn(pipelineManagementApi, 'getPipelineNode').mockResolvedValue({
			success: true,
			data: {
				last_updated: '2026-10-22T00:00:00Z',
				content: { kind: 'processor_group', data: { processors: [] } }
			}
		} as any)

		vi.spyOn(pipelineManagementApi, 'updatePipelineNode').mockResolvedValue({
			success: true,
			data: {}
		} as any)
	})

	it('initializes processor node with empty processors array and 0 count', () => {
		const store = usePipelineBuildStore()
		const procNode = store.addProcessorNode(100, 200, 'My Processor Group')

		expect(procNode).toBeDefined()
		expect(procNode.type).toBe('processor')
		expect(procNode.label).toBe('My Processor Group')
		expect(procNode.data.processors).toEqual([])
		expect(procNode.data.processCount).toBe(0)
		expect(procNode.data.activeCount).toBe(0)
	})

	it('adds a processor to processor group and syncs counts', () => {
		const store = usePipelineBuildStore()
		const procNode = store.addProcessorNode()

		store.addProcessorToGroup(procNode.id, {
			processor_id: 'p-1',
			name: 'Processor 1',
			module: 'null',
			status: true
		})

		expect(procNode.data.processors?.length).toBe(1)
		expect(procNode.data.processCount).toBe(1)
		expect(procNode.data.activeCount).toBe(1)

		store.addProcessorToGroup(procNode.id, {
			processor_id: 'p-2',
			name: 'Processor 2',
			module: 'attribute_filter',
			status: false
		})

		expect(procNode.data.processors?.length).toBe(2)
		expect(procNode.data.processCount).toBe(2)
		expect(procNode.data.activeCount).toBe(1)
	})

	it('updates processor in group', () => {
		const store = usePipelineBuildStore()
		const procNode = store.addProcessorNode()

		store.addProcessorToGroup(procNode.id, {
			processor_id: 'p-1',
			name: 'Processor 1',
			module: 'null',
			status: true
		})

		store.updateProcessorInGroup(procNode.id, 'p-1', {
			name: 'Updated Processor Name',
			module: 'drop_attributes'
		})

		const item = procNode.data.processors?.find((p) => p.processor_id === 'p-1')
		expect(item?.name).toBe('Updated Processor Name')
		expect(item?.module).toBe('drop_attributes')
	})

	it('toggles processor active status and updates activeCount', () => {
		const store = usePipelineBuildStore()
		const procNode = store.addProcessorNode()

		store.addProcessorToGroup(procNode.id, {
			processor_id: 'p-1',
			name: 'Processor 1',
			module: 'null',
			status: true
		})

		expect(procNode.data.activeCount).toBe(1)

		// Toggle to disabled
		store.toggleProcessorStatus(procNode.id, 'p-1')
		expect(procNode.data.processors?.[0].status).toBe(false)
		expect(procNode.data.activeCount).toBe(0)

		// Toggle back to enabled
		store.toggleProcessorStatus(procNode.id, 'p-1')
		expect(procNode.data.processors?.[0].status).toBe(true)
		expect(procNode.data.activeCount).toBe(1)
	})

	it('reorders processors in sequence', () => {
		const store = usePipelineBuildStore()
		const procNode = store.addProcessorNode()

		store.addProcessorToGroup(procNode.id, {
			processor_id: 'p-1',
			name: 'First',
			module: 'null'
		})
		store.addProcessorToGroup(procNode.id, {
			processor_id: 'p-2',
			name: 'Second',
			module: 'null'
		})
		store.addProcessorToGroup(procNode.id, {
			processor_id: 'p-3',
			name: 'Third',
			module: 'null'
		})

		// Move 'First' (index 0) to index 2
		store.reorderProcessorGroup(procNode.id, 0, 2)

		expect(procNode.data.processors?.map((p) => p.name)).toEqual(['Second', 'Third', 'First'])
	})

	it('bulk disables and bulk deletes processors', () => {
		const store = usePipelineBuildStore()
		const procNode = store.addProcessorNode()

		store.addProcessorToGroup(procNode.id, {
			processor_id: 'p-1',
			name: 'P1',
			module: 'null',
			status: true
		})
		store.addProcessorToGroup(procNode.id, {
			processor_id: 'p-2',
			name: 'P2',
			module: 'null',
			status: true
		})
		store.addProcessorToGroup(procNode.id, {
			processor_id: 'p-3',
			name: 'P3',
			module: 'null',
			status: true
		})

		// Bulk disable p-1 and p-2
		store.bulkDisableProcessors(procNode.id, ['p-1', 'p-2'])
		expect(procNode.data.activeCount).toBe(1)
		expect(procNode.data.processors?.[0].status).toBe(false)
		expect(procNode.data.processors?.[1].status).toBe(false)
		expect(procNode.data.processors?.[2].status).toBe(true)

		// Bulk delete p-1 and p-3
		store.bulkDeleteProcessors(procNode.id, ['p-1', 'p-3'])
		expect(procNode.data.processors?.length).toBe(1)
		expect(procNode.data.processors?.[0].processor_id).toBe('p-2')
		expect(procNode.data.processCount).toBe(1)
	})

	it('EdgeItemsModal Select All selects all items and deselects all on second click', async () => {
		const store = usePipelineBuildStore()
		store.edgeItemModal.isOpen = true
		store.edgeItemModal.edgeId = 'edge-1'
		store.pipelineItems = [
			{ item_id: 'i-1', name: 'Item 1', host_id: 'host-1' },
			{ item_id: 'i-2', name: 'Item 2', host_id: 'host-1' }
		]

		const wrapper = mount(EdgeItemsModal, {
			global: {
				stubs: {
					'b-checkbox': {
						name: 'BCheckbox',
						template:
							'<div class="b-checkbox-stub"><input type="checkbox" :checked="checked !== undefined ? checked : modelValue" /></div>',
						props: ['modelValue', 'checked']
					},
					AppInputText: true,
					AppInputSelect: true
				}
			}
		})

		await flushPromises()

		const selectAllContainer = wrapper.find('.group.cursor-pointer')
		expect(selectAllContainer.exists()).toBe(true)

		// 1. Click Select All
		await selectAllContainer.trigger('click')
		await flushPromises()
		expect(wrapper.text()).toContain('2 items selected')

		// 2. Click Select All again to deselect
		await selectAllContainer.trigger('click')
		await flushPromises()
		expect(wrapper.text()).toContain('0 items selected')

		// 3. Click individual item row
		const rows = wrapper.findAll('.cursor-pointer')
		// rows[0] is selectAllContainer, rows[1] is item 1
		await rows[1].trigger('click')
		await flushPromises()
		expect(wrapper.text()).toContain('1 items selected')
	})

	it('EdgeItemsModal correctly checks checkboxes for pre-selected items on open', async () => {
		const store = usePipelineBuildStore()
		const sourceNode = store.addSourceNode(0, 0)
		const procNode = store.addProcessorNode(200, 0, 'Group 1')
		store.pipelineItems = [
			{ item_id: 'i-1', name: 'Item 1', host_id: 'host-1' },
			{ item_id: 'i-2', name: 'Item 2', host_id: 'host-1' },
			{ item_id: 'i-3', name: 'Item 3', host_id: 'host-1' }
		]

		const edgeId = store.connectNodes(sourceNode.id, procNode.id)
		const edge = store.edges.find((e) => e.id === edgeId)
		if (edge) {
			edge.data = {
				...edge.data,
				selectedItemIds: ['i-1', { item_id: 'i-3', name: 'Item 3' }]
			}
		}

		store.edgeItemModal.edgeId = edgeId
		store.edgeItemModal.isOpen = true

		const wrapper = mount(EdgeItemsModal, {
			global: {
				stubs: {
					UiCheckbox: {
						name: 'UiCheckbox',
						template:
							'<div class="b-checkbox-stub"><input type="checkbox" :checked="checked === true || (checked === undefined && Boolean(modelValue))" /></div>',
						props: ['modelValue', 'checked']
					},
					'ui-checkbox': {
						name: 'UiCheckbox',
						template:
							'<div class="b-checkbox-stub"><input type="checkbox" :checked="checked === true || (checked === undefined && Boolean(modelValue))" /></div>',
						props: ['modelValue', 'checked']
					},
					'b-checkbox': {
						name: 'BCheckbox',
						template:
							'<div class="b-checkbox-stub"><input type="checkbox" :checked="checked === true || (checked === undefined && Boolean(modelValue))" /></div>',
						props: ['modelValue', 'checked']
					},
					AppInputText: true,
					AppInputSelect: true
				}
			}
		})

		await flushPromises()

		expect(wrapper.text()).toContain('2 items selected')

		const checkboxInputs = wrapper.findAll('.b-checkbox-stub input[type="checkbox"]')
		expect(checkboxInputs.length).toBe(4)

		const selectAllInput = checkboxInputs[0].element as HTMLInputElement
		const item1Input = checkboxInputs[1].element as HTMLInputElement
		const item2Input = checkboxInputs[2].element as HTMLInputElement
		const item3Input = checkboxInputs[3].element as HTMLInputElement

		expect(selectAllInput.checked).toBe(false)
		expect(item1Input.checked).toBe(true)
		expect(item2Input.checked).toBe(false)
		expect(item3Input.checked).toBe(true)

		const saveButton = wrapper
			.findAll('button')
			.find((btn) => btn.text().includes('Apply') || btn.text().includes('Save'))
		if (saveButton) {
			await saveButton.trigger('click')
			await flushPromises()
			expect(store.edgeItemModal.isOpen).toBe(false)
			expect(edge?.data.selectedItemIds).toEqual(['i-1', 'i-3'])
			expect(edge?.data.label).toBe('2 Items')
		}
	})

	it('EdgeItemsModal retains 0 items selected when edge.data.selectedItemIds is initially empty', async () => {
		const store = usePipelineBuildStore()
		const sourceNode = store.addSourceNode(0, 0)
		sourceNode.data.items = [
			{ item_id: 'i-1', name: 'Item 1' },
			{ item_id: 'i-2', name: 'Item 2' }
		]
		const procNode = store.addProcessorNode(200, 0, 'Group 1')
		store.pipelineItems = [
			{ item_id: 'i-1', name: 'Item 1', host_id: 'host-1' },
			{ item_id: 'i-2', name: 'Item 2', host_id: 'host-1' },
			{ item_id: 'i-3', name: 'Item 3', host_id: 'host-1' }
		]

		const edgeId = store.connectNodes(sourceNode.id, procNode.id)
		const edge = store.edges.find((e) => e.id === edgeId)
		if (edge) {
			edge.data = {
				...edge.data,
				label: '0 Items',
				selectedItemIds: []
			}
		}

		store.openEdgeItemSelector(edgeId)

		const wrapper = mount(EdgeItemsModal, {
			global: {
				stubs: {
					UiCheckbox: {
						name: 'UiCheckbox',
						template:
							'<div class="b-checkbox-stub"><input type="checkbox" :checked="checked === true || (checked === undefined && Boolean(modelValue))" /></div>',
						props: ['modelValue', 'checked', 'color']
					},
					'ui-checkbox': {
						name: 'UiCheckbox',
						template:
							'<div class="b-checkbox-stub"><input type="checkbox" :checked="checked === true || (checked === undefined && Boolean(modelValue))" /></div>',
						props: ['modelValue', 'checked', 'color']
					},
					'b-checkbox': {
						name: 'BCheckbox',
						template:
							'<div class="b-checkbox-stub"><input type="checkbox" :checked="checked === true || (checked === undefined && Boolean(modelValue))" /></div>',
						props: ['modelValue', 'checked', 'color']
					},
					AppInputText: true,
					AppInputSelect: true
				}
			}
		})

		await flushPromises()

		expect(wrapper.text()).toContain('0 items selected')

		const checkboxInputs = wrapper.findAll('.b-checkbox-stub input[type="checkbox"]')
		expect(checkboxInputs.length).toBe(4)

		const item1Input = checkboxInputs[1].element as HTMLInputElement
		const item2Input = checkboxInputs[2].element as HTMLInputElement
		const item3Input = checkboxInputs[3].element as HTMLInputElement

		expect(item1Input.checked).toBe(false)
		expect(item2Input.checked).toBe(false)
		expect(item3Input.checked).toBe(false)
	})

	it('ProcessorGroupDrawer separates View Detail and Edit Processor in 3-dots action menu', async () => {
		const store = usePipelineBuildStore()
		const processorStore = useProcessorStore()

		const procNode = store.addProcessorNode(100, 200, 'Test Group')
		store.selectedNode = procNode
		store.addProcessorToGroup(procNode.id, {
			processor_id: 'proc-detail-1',
			name: 'Processor Sample',
			module: 'attribute_filter',
			status: true
		})

		vi.spyOn(processorsApi, 'get').mockResolvedValue({
			success: true,
			data: {
				processors: [
					{
						processor_id: 'proc-detail-1',
						name: 'Processor Sample',
						adm_status: 1,
						last_updated: '2026-10-22T00:00:00Z',
						config: {
							module: 'attribute_filter',
							config: { match: {} }
						}
					} as any
				],
				total: 1
			}
		} as any)

		const wrapper = mount(ProcessorGroupDrawer, {
			global: {
				stubs: {
					'b-checkbox': {
						name: 'BCheckbox',
						template:
							'<div class="b-checkbox-stub"><input type="checkbox" :checked="checked !== undefined ? checked : modelValue" /></div>',
						props: ['modelValue', 'checked']
					},
					AppInputText: true
				}
			}
		})

		await flushPromises()

		// 1. Open 3-dots action menu
		const actionButton = wrapper.find('button[title="Actions"]')
		expect(actionButton.exists()).toBe(true)
		await actionButton.trigger('click')
		await flushPromises()

		// 2. Expect 4 action menu items are rendered
		expect(wrapper.text()).toContain('Preview Data')
		expect(wrapper.text()).toContain('View / Edit')
		expect(wrapper.text()).toContain('Disable')
		expect(wrapper.text()).toContain('Delete')

		// 3. Click "Preview Data" from dropdown
		const menuButtons = wrapper.findAll('.shadow-xl button')
		const previewBtn = menuButtons.find((b) => b.text().includes('Preview Data'))
		expect(previewBtn).toBeDefined()

		await previewBtn!.trigger('click')
		await flushPromises()

		expect(store.previewDataModal.isOpen).toBe(true)
		expect(store.previewDataModal.processorId).toBe('proc-detail-1')

		// 4. Open menu again and click "View / Edit"
		await actionButton.trigger('click')
		await flushPromises()

		const updatedMenuButtons = wrapper.findAll('.shadow-xl button')
		const viewEditBtn = updatedMenuButtons.find((b) => b.text().includes('View / Edit'))
		expect(viewEditBtn).toBeDefined()

		await viewEditBtn!.trigger('click')
		await flushPromises()

		expect(processorStore.processorModalShow).toBe(true)
		expect(processorStore.currentProcessor?.processor_id).toBe('proc-detail-1')
	})

	it('ProcessorGroupDrawer Add Processor opens modal form and adds processor in-memory via custom handler', async () => {
		const store = usePipelineBuildStore()
		const processorStore = useProcessorStore()

		const procNode = store.addProcessorNode(100, 200, 'Test Group')
		store.selectedNode = procNode

		const wrapper = mount(ProcessorGroupDrawer, {
			global: {
				stubs: {
					'b-checkbox': true,
					AppInputText: true
				}
			}
		})

		// Click "Add Processor"
		const buttons = wrapper.findAll('button')
		const addBtn = buttons.find((b) => b.text().includes('Add Processor'))
		expect(addBtn).toBeDefined()
		await addBtn!.trigger('click')
		await flushPromises()

		expect(processorStore.processorModalShow).toBe(true)
		expect(processorStore.processorFormMode).toBe(FormMode.ADD)
		expect(typeof processorStore.customSubmitHandler).toBe('function')
		expect(processorStore.hideInputItemsAndProcessors).toBe(true)
		expect(processorStore.lockedCluster).toBeDefined()

		// Simulate form submit
		await processorStore.customSubmitHandler!({
			name: 'Brand New In-Memory Processor',
			adm_status: 1,
			config: {
				type: 'drop_attributes',
				config: { keys: { record: ['token'] } }
			}
		})

		expect(procNode.data.processors?.length).toBe(1)
		const created = procNode.data.processors![0]
		expect(created.name).toBe('Brand New In-Memory Processor')
		expect(created.module).toBe('drop_attributes')
		expect(created.processor_id).toBeDefined()
		expect(created.processor_id).toMatch(
			/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i
		)
	})

	it('Processor Form locks cluster (read-only) and hides input_items & input_processors when hideInputItemsAndProcessors is enabled', async () => {
		const pinia = createPinia()
		setActivePinia(pinia)
		const processorStore = useProcessorStore()
		let submittedPayload: any = null

		processorStore.toggleProcessorModalForm(
			true,
			FormMode.ADD,
			null,
			async (payload: any) => {
				submittedPayload = payload
				return true
			},
			{
				lockedCluster: 'http://test-pipeline-cluster.local:8080',
				hideInputItemsAndProcessors: true
			}
		)

		const wrapper = mount(Form, {
			props: {
				showCancel: true
			},
			global: {
				plugins: [pinia],
				stubs: {
					'b-card': {
						template: '<div><slot name="header"/><slot/><slot name="footer"/></div>'
					},
					'b-icon': true,
					'b-heading': true,
					'b-flex': { template: '<div><slot/></div>' },
					'b-flex-item': { template: '<div><slot/></div>' },
					'b-button': {
						template: '<button type="button"><slot/></button>'
					},
					AppInputCluster: {
						name: 'AppInputCluster',
						template:
							'<div class="app-input-cluster-stub" :data-disabled="disabled" :data-cluster="modelValue"></div>',
						props: ['modelValue', 'disabled']
					},
					AppInputItem: {
						name: 'AppInputItem',
						template: '<div class="app-input-item-stub"></div>'
					},
					AppInputProcessor: {
						name: 'AppInputProcessor',
						template: '<div class="app-input-processor-stub"></div>'
					},
					AppInputText: true
				}
			}
		})

		await flushPromises()

		// 1. Cluster input is disabled and matches lockedCluster
		const clusterStub = wrapper.find('.app-input-cluster-stub')
		expect(clusterStub.exists()).toBe(true)
		expect(clusterStub.attributes('data-disabled')).toBe('true')
		expect(clusterStub.attributes('data-cluster')).toBe(
			'http://test-pipeline-cluster.local:8080'
		)

		// 2. Input items and input processors slots are NOT rendered
		expect(wrapper.find('.app-input-item-stub').exists()).toBe(false)
		expect(wrapper.find('.app-input-processor-stub').exists()).toBe(false)

		// 3. Trigger submit via DynamicZodForm
		const dynamicForm = wrapper.findComponent({ name: 'DynamicZodForm' })
		dynamicForm.vm.$emit('submit', {
			name: 'Test Processor',
			adm_status: 1,
			input_items: ['item-1'],
			input_processors: ['proc-1'],
			config: {
				type: 'drop_attributes',
				config: {}
			}
		})
		await flushPromises()

		expect(submittedPayload).not.toBeNull()
		expect(submittedPayload.input_items).toEqual([])
		expect(submittedPayload.input_processors).toEqual([])
	})

	it('BuilderProcessorNode + Processor button opens modal form and adds processor into processor group', async () => {
		const store = usePipelineBuildStore()
		const processorStore = useProcessorStore()

		const procNode = store.addProcessorNode(100, 200, 'Test Node Group')

		const wrapper = mount(BuilderProcessorNode, {
			props: {
				id: procNode.id,
				data: procNode.data
			},
			global: {
				stubs: {
					Handle: true
				}
			}
		})

		await flushPromises()

		// Find bottom button "+ Processor"
		const buttons = wrapper.findAll('button')
		const procBtn = buttons.find((b) => b.text().trim() === 'Processor')
		expect(procBtn).toBeDefined()

		await procBtn!.trigger('click')
		await flushPromises()

		expect(store.selectedNode?.id).toBe(procNode.id)
		expect(processorStore.processorModalShow).toBe(true)
		expect(processorStore.processorFormMode).toBe(FormMode.ADD)
		expect(typeof processorStore.customSubmitHandler).toBe('function')
		expect(processorStore.hideInputItemsAndProcessors).toBe(true)

		// Submit form
		await processorStore.customSubmitHandler!({
			name: 'Sub-Processor Added From Node',
			adm_status: 1,
			config: {
				type: 'attribute_filter',
				config: {}
			}
		})

		expect(procNode.data.processors?.length).toBe(1)
		expect(procNode.data.processors![0].name).toBe('Sub-Processor Added From Node')
		expect(procNode.data.processors![0].module).toBe('attribute_filter')
	})

	it('BuilderCanvas deletes selected nodes via keyboard Delete/Backspace key and ignores when typing in input', async () => {
		const store = usePipelineBuildStore()
		const node1 = store.addProcessorNode(100, 100, 'To Delete Node')
		const node2 = store.addProcessorNode(300, 100, 'Keep Node')
		store.selectedNode = node1

		const wrapper = mount(BuilderCanvas, {
			global: {
				stubs: {
					VueFlow: {
						template: '<div class="vue-flow-stub"><slot/></div>'
					},
					Background: true,
					CanvasBottomControls: true,
					CanvasNodeToolbar: true,
					CanvasEmptyState: true,
					EdgeItemsModal: true,
					SourceNode: true,
					BuilderProcessorNode: true,
					BuilderSinkNode: true,
					BuilderEdge: true
				}
			}
		})

		await flushPromises()

		// 1. Press Backspace while focused inside an input -> node should NOT be deleted
		const inputElem = document.createElement('input')
		document.body.appendChild(inputElem)
		inputElem.focus()

		const inputEvent = new KeyboardEvent('keydown', { key: 'Backspace', bubbles: true })
		inputElem.dispatchEvent(inputEvent)
		await flushPromises()

		expect(store.nodes.find((n: any) => n.id === node1.id)).toBeDefined()

		// 2. Press Delete on window when node is selected -> node SHOULD be deleted
		document.body.removeChild(inputElem)
		const deleteEvent = new KeyboardEvent('keydown', { key: 'Delete', bubbles: true })
		window.dispatchEvent(deleteEvent)
		await flushPromises()

		expect(store.nodes.find((n: any) => n.id === node1.id)).toBeUndefined()
		expect(store.nodes.find((n: any) => n.id === node2.id)).toBeDefined()
		expect(store.selectedNode).toBeUndefined()

		wrapper.unmount()
	})

	it('autoLayout arranges nodes horizontally along connection lines with spacious separation', () => {
		const store = usePipelineBuildStore()
		store.nodes = []
		store.edges = []

		const source = store.addSourceNode(500, 500)
		const proc1 = store.addProcessorNode(0, 0, 'Proc 1')
		const proc2 = store.addProcessorNode(10, 10, 'Proc 2')
		const sink = store.addSinkNode(20, 20)

		// Connect: Source -> Proc 1 -> Proc 2 -> Sink
		store.connectNodes(source.id, proc1.id)
		store.connectNodes(proc1.id, proc2.id)
		store.connectNodes(proc2.id, sink.id)

		store.autoLayout()

		// 1. Order along connection lines: Source.x < Proc1.x < Proc2.x < Sink.x
		expect(source.position.x).toBeLessThan(proc1.position.x)
		expect(proc1.position.x).toBeLessThan(proc2.position.x)
		expect(proc2.position.x).toBeLessThan(sink.position.x)

		// 2. Wide horizontal separation (>= 250px)
		expect(proc1.position.x - source.position.x).toBeGreaterThanOrEqual(250)
		expect(proc2.position.x - proc1.position.x).toBeGreaterThanOrEqual(250)
		expect(sink.position.x - proc2.position.x).toBeGreaterThanOrEqual(250)
	})

	it('autoLayout places parallel branches on separate Y levels without overlap', () => {
		const store = usePipelineBuildStore()
		store.nodes = []
		store.edges = []

		const source = store.addSourceNode(0, 0)
		const procA = store.addProcessorNode(0, 0, 'Branch A')
		const procB = store.addProcessorNode(0, 0, 'Branch B')

		// Parallel branches from Source: Source -> ProcA, Source -> ProcB
		store.connectNodes(source.id, procA.id)
		store.connectNodes(source.id, procB.id)

		store.autoLayout()

		// Both branches should be to the right of source
		expect(procA.position.x).toBeGreaterThanOrEqual(source.position.x + 250)
		expect(procB.position.x).toBeGreaterThanOrEqual(source.position.x + 250)

		// ProcA and ProcB must be placed on distinct Y levels with vertical spacing >= 100px
		const verticalDistance = Math.abs(procA.position.y - procB.position.y)
		expect(verticalDistance).toBeGreaterThanOrEqual(100)
	})

	it('renders selected sink name instead of kafka in BuilderSinkNode component', () => {
		const store = usePipelineBuildStore()
		store.availableSinks = [{ sink_id: 'sink-1', name: 'SINK 1', type: 'kafka' }] as any

		// Case 1: unselected sink node
		const wrapperUnselected = mount(BuilderSinkNode, {
			props: {
				id: 'sink-node-1',
				data: {
					id: 'sink-node-1',
					name: 'Sink Target',
					type: 'No sink selected'
				}
			},
			global: {
				stubs: {
					Handle: true
				}
			}
		})
		expect(wrapperUnselected.text()).toContain('Sink Target')
		expect(wrapperUnselected.text()).toContain('No sink selected')
		expect(wrapperUnselected.text()).not.toContain('Kafka')

		// Case 2: sink selected
		const wrapperSelected = mount(BuilderSinkNode, {
			props: {
				id: 'sink-node-2',
				data: {
					id: 'sink-node-2',
					sinkId: 'sink-1',
					name: 'Sink Target',
					sinkName: 'SINK 1',
					type: 'kafka'
				}
			},
			global: {
				stubs: {
					Handle: true
				}
			}
		})
		expect(wrapperSelected.text()).toContain('Sink Target')
		expect(wrapperSelected.text()).toContain('SINK 1')
		expect(wrapperSelected.text()).not.toContain('Kafka')
	})

	it('auto-matches items and suppresses modal when source already has another processor group connection', () => {
		const store = usePipelineBuildStore()
		store.nodes = []
		store.edges = []
		store.edgeItemModal.isOpen = false

		const source = store.addSourceNode(100, 100)
		source.data.items = ['item-alpha', 'item-beta', 'item-gamma']
		source.data.itemCount = 3
		source.data.item_count = 3

		const proc1 = store.addProcessorNode(350, 100, 'Processor Group 1')
		const proc2 = store.addProcessorNode(350, 250, 'Processor Group 2')

		// 1. First connection from source to proc1 -> opens modal, starts with 0 Items
		const edge1Id = store.connectNodes(source.id, proc1.id)
		expect(store.edgeItemModal.isOpen).toBe(true)
		expect(store.edgeItemModal.edgeId).toBe(edge1Id)

		// Close modal and apply items
		store.closeEdgeItemSelector()
		store.applyEdgeItems(edge1Id, ['item-alpha', 'item-beta', 'item-gamma'])

		// 2. Second connection from source to proc2 -> must NOT open modal, and auto-match items from source
		const edge2Id = store.connectNodes(source.id, proc2.id)
		expect(store.edgeItemModal.isOpen).toBe(false)

		const edge2 = store.edges.find((e) => e.id === edge2Id)
		expect(edge2?.data?.label).toBe('3 Items')
		expect(edge2?.data?.selectedItemIds).toEqual(['item-alpha', 'item-beta', 'item-gamma'])
		expect(proc2.data.itemCount).toBe(3)
	})

	it('marks store as dirty when node is dragged and dropped (node-drag-stop)', () => {
		const store = usePipelineBuildStore()
		store.isDirty = false
		expect(store.isDirty).toBe(false)

		const wrapper = mount(BuilderCanvas, {
			global: {
				stubs: {
					VueFlow: {
						name: 'VueFlow',
						template: '<div class="vue-flow-stub"><slot /></div>',
						emits: ['node-drag-stop']
					},
					Background: true,
					CanvasBottomControls: true,
					CanvasNodeToolbar: true,
					CanvasEmptyState: true,
					EdgeItemsModal: true,
					SourceNode: true,
					BuilderProcessorNode: true,
					BuilderSinkNode: true,
					BuilderEdge: true
				}
			}
		})

		const vueFlowComponent = wrapper.findComponent({ name: 'VueFlow' })
		expect(vueFlowComponent.exists()).toBe(true)
		vueFlowComponent.vm.$emit('node-drag-stop')

		expect(store.isDirty).toBe(true)
	})

	it('loads processor group with disabled status (adm_status: 2) on reload and correctly updates activeCount', async () => {
		const store = usePipelineBuildStore()
		const procNodeId = 'proc-group-node-101'

		vi.spyOn(pipelineManagementApi, 'getPipelineById').mockResolvedValue({
			success: true,
			data: {
				pipeline_management_id: 'pipe-reload-1',
				canvases: [{ canvas_id: 'canvas-reload-1' }]
			}
		} as any)

		vi.spyOn(pipelineManagementApi, 'getPipelineGraph').mockResolvedValue({
			success: true,
			data: {
				canvas_id: 'canvas-reload-1',
				graph: {
					nodes: [
						{
							node_id: procNodeId,
							kind: 'processor_group',
							position_x: 200,
							position_y: 200,
							data: {
								name: 'Filter Pipeline Group',
								processors: [
									{
										processor_id: 'p-1',
										name: 'Active Filter',
										module: 'attribute_filter'
									},
									{
										processor_id: 'p-2',
										name: 'Disabled Drop',
										module: 'drop_attributes'
									}
								]
							}
						}
					],
					relations: []
				}
			}
		} as any)

		// getPipelineNode returns real content with adm_status: 1 (active) and 2 (disabled)
		vi.spyOn(pipelineManagementApi, 'getPipelineNode').mockResolvedValue({
			success: true,
			data: {
				node_id: procNodeId,
				kind: 'processor_group',
				data: {
					processors: [
						{
							processor_id: 'p-1',
							name: 'Active Filter',
							adm_status: 1,
							config: { type: 'attribute_filter' }
						},
						{
							processor_id: 'p-2',
							name: 'Disabled Drop',
							adm_status: 2,
							config: { type: 'drop_attributes' }
						}
					]
				}
			}
		} as any)

		await store.loadBuilder('pipe-reload-1', 'canvas-reload-1')

		const procNode = store.nodes.find((n) => n.id === procNodeId)
		expect(procNode).toBeDefined()
		expect(procNode?.data.processors?.length).toBe(2)

		const p1 = procNode?.data.processors?.find((p: any) => p.processor_id === 'p-1')
		const p2 = procNode?.data.processors?.find((p: any) => p.processor_id === 'p-2')

		expect(p1?.status).toBe(true)
		expect(p2?.status).toBe(false)
		expect(procNode?.data.activeCount).toBe(1)
	})

	it('renders Disabled badge and line-through text in ProcessorGroupDrawer for disabled processor', () => {
		const store = usePipelineBuildStore()

		const procNode = {
			id: 'proc-drawer-test',
			type: 'processor',
			label: 'Transform Group',
			data: {
				name: 'Transform Group',
				processors: [
					{
						processor_id: 'proc-active',
						name: 'Active Processor',
						module: 'attribute_filter',
						status: true
					},
					{
						processor_id: 'proc-disabled',
						name: 'Disabled Processor',
						module: 'drop_attributes',
						status: false
					}
				]
			}
		}

		store.selectedNode = procNode as any

		const wrapper = mount(ProcessorGroupDrawer, {
			global: {
				stubs: {
					AppInputText: true,
					'b-checkbox': true
				}
			}
		})

		const textContent = wrapper.text()
		expect(textContent).toContain('Active Processor')
		expect(textContent).toContain('Disabled Processor')
		// Verify Disabled badge is displayed
		expect(textContent).toContain('Disabled')

		// Verify disabled text styling
		const nameSpans = wrapper.findAll('span.text-xs.font-medium')
		const disabledSpan = nameSpans.find((el) => el.text().includes('Disabled Processor'))
		expect(disabledSpan).toBeDefined()
		expect(disabledSpan?.classes()).toContain('line-through')
	})

	it('renders PreviewDataModal with processor sequence and switches target processor on click', async () => {
		const pinia = createPinia()
		setActivePinia(pinia)

		const store = usePipelineBuildStore(pinia)
		store.selectedNode = {
			id: 'proc-group-node-1',
			type: 'processor',
			data: {
				processors: [
					{
						processor_id: 'proc-1',
						name: 'Filter Processor',
						module: 'attribute_filter',
						status: true
					},
					{
						processor_id: 'proc-2',
						name: 'Drop Processor',
						module: 'drop_attributes',
						status: true
					}
				]
			}
		} as any

		store.openPreviewDataModal('proc-group-node-1', 'proc-1')

		const wrapper = mount(PreviewDataModal, {
			global: {
				plugins: [pinia]
			}
		})

		await wrapper.vm.$nextTick()

		// Modal should be visible
		expect(wrapper.text()).toContain('Preview Data Mode')
		expect(wrapper.text()).toContain('Processors in Sequence')
		expect(wrapper.text()).toContain('Record Before')
		expect(wrapper.text()).toContain('Record After')
		expect(wrapper.text()).toContain('Add Processor')
		expect(wrapper.text()).toContain('Select All Processor')

		// Processors rendered in sequence
		expect(wrapper.text()).toContain('Filter Processor')
		expect(wrapper.text()).toContain('Drop Processor')

		// Initially target is proc-1
		expect(wrapper.text()).toContain('Target')

		// Click on second processor card
		const procCards = wrapper
			.findAll('.cursor-pointer')
			.filter((card) => card.text().includes('Drop Processor'))
		expect(procCards.length).toBeGreaterThan(0)
		await procCards[0].trigger('click')
		await wrapper.vm.$nextTick()

		// Target should now be proc-2
		expect((wrapper.vm as any).selectedProcessorId).toBe('proc-2')

		// Simulate captured records arriving via WebSocket
		;(wrapper.vm as any).capturedRecords = [
			{
				id: 1,
				time: '13:41:02',
				status: '200 OK',
				statusType: 'success',
				message_in: { user_id: 1024, event: 'click' },
				message_out: { user_id: '[MASKED]', event: 'click' }
			}
		]
		await wrapper.vm.$nextTick()

		// In list view, Record #1 should be visible
		expect(wrapper.text()).toContain('Record #1')
		expect(wrapper.text()).toContain('13:41:02')

		// Click on the record card to enter detail view
		const recordCard = wrapper
			.findAll('.cursor-pointer')
			.find((el) => el.text().includes('Record #1'))
		expect(recordCard).toBeDefined()
		await recordCard?.trigger('click')
		await wrapper.vm.$nextTick()

		// In detail view, Back to List should appear
		expect(wrapper.text()).toContain('Back to List')
		expect(wrapper.text()).toContain('Viewing Record #1')
		expect(wrapper.text()).toContain('[MASKED]')

		// Click Back to List to return to list mode
		const backBtn = wrapper.findAll('button').find((b) => b.text().includes('Back to List'))
		expect(backBtn).toBeDefined()
		await backBtn?.trigger('click')
		await wrapper.vm.$nextTick()

		// Now Back to List is hidden, Record #1 card is visible again
		expect(wrapper.text()).not.toContain('Viewing Record #1')

		// Test Data Not Found state when capturedRecords is empty and isPooling is false
		;(wrapper.vm as any).capturedRecords = []
		;(wrapper.vm as any).isPooling = false
		await wrapper.vm.$nextTick()
		expect(wrapper.text()).toContain('Sorry, Data not found.')
		expect(wrapper.text()).toContain(
			'Check your configuration filters or wait for fresh polling entries.'
		)

		// Close modal
		const closeBtn = wrapper.find('button[title="Close Modal"]')
		await closeBtn.trigger('click')
		expect(store.previewDataModal.isOpen).toBe(false)
	})
})

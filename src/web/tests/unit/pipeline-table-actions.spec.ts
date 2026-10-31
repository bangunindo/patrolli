// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import '@@/shared/types/array.type'
import { describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import PipelineTable from '@/components/pipeline/Table.vue'
import { usePipelineStore } from '@/stores/pipeline.store'
import type { IPipelineItem } from '@@/shared/types/pipeline.management.type'
import pipelineManagementApi from '@/app/api/pipeline.management.api'

describe('Pipeline Table & Host Selector Enhancements', () => {
	const dummyPipelines: IPipelineItem[] = [
		{
			id: 'pipe-1',
			name: 'Network Telemetry Pipeline',
			code: 'pipe-net-tel-01',
			status: 'running',
			access: 'public',
			assignedHostsCount: 4,
			assignedSinksCount: 2,
			lastUpdate: '2 mins ago',
			assignedHosts: ['h1', 'h2'],
			assignedSinks: ['s1'],
			cluster_url: 'http://test-cluster'
		},
		{
			id: 'pipe-2',
			name: 'Infrastructure Log Pipeline',
			code: 'pipe-inf-log-07',
			status: 'stopped',
			access: 'private',
			assignedHostsCount: 8,
			assignedSinksCount: 3,
			lastUpdate: '1 hour ago',
			assignedHosts: ['h3'],
			assignedSinks: ['s2'],
			cluster_url: 'http://test-cluster'
		},
		{
			id: 'pipe-3',
			name: 'App Metrics Pipeline',
			code: 'pipe-app-met-03',
			status: 'draft',
			access: 'public',
			assignedHostsCount: 2,
			assignedSinksCount: 1,
			lastUpdate: 'yesterday',
			assignedHosts: [],
			assignedSinks: [],
			cluster_url: 'http://test-cluster'
		}
	]

	const commonStubs = {
		AppInputText: {
			props: ['modelValue', 'placeholder'],
			template:
				'<input :value="modelValue" @input="$emit(\'update:modelValue\', $event.target.value)" />'
		},
		'app-table': {
			props: ['columns', 'data', 'selected'],
			template: `
				<div v-if="data?.length" class="mock-app-table">
					<slot name="col.name" :data="data[0]" />
					<slot name="col.status" :data="data[0]" />
					<slot name="col.access" :data="data[0]" />
					<slot name="col.assigned_resources" :data="data[0]" />
					<slot name="col.action" :data="data[0]" />
				</div>
			`
		},
		'b-dropdown': { template: '<div><slot/></div>' },
		'b-dropdown-toggle': { template: '<div><slot/></div>' },
		'b-dropdown-content': { template: '<div><slot/></div>' },
		'b-button': {
			props: ['iconLeft', 'color', 'variant'],
			template: '<button type="button" @click="$emit(\'click\')"><slot/></button>'
		},
		'b-icon': true,
		'b-tag': true,
		'b-modal': {
			props: ['modelValue'],
			template:
				'<div v-if="modelValue" class="mock-modal"><slot name="header"/><slot/><slot name="footer"/></div>'
		},
		'b-modal-header': { template: '<div><slot/></div>' },
		'b-modal-body': { template: '<div><slot/></div>' },
		'b-modal-footer': { template: '<div><slot/></div>' },
		'b-heading': true,
		'modal-confirmation-delete': true
	}

	it('startSelected menyalakan pipeline yang belum running', async () => {
		const pinia = createPinia()
		setActivePinia(pinia)
		const store = usePipelineStore()
		store.pipelines = JSON.parse(JSON.stringify(dummyPipelines))

		vi.spyOn(pipelineManagementApi, 'getPipelineById').mockResolvedValue({
			success: true,
			data: {
				name: 'Test',
				visibility: 1,
				host_ids: [],
				sink_ids: [],
				last_updated: '2026-10-22'
			}
		} as any)
		const updateSpy = vi.spyOn(pipelineManagementApi, 'updatePipeline').mockResolvedValue({
			success: true
		} as any)

		await store.startSelected(['pipe-2', 'pipe-3'])

		expect(updateSpy).toHaveBeenCalledWith(
			'pipe-2',
			expect.objectContaining({ adm_status: 1 }),
			['http://test-cluster']
		)
		expect(updateSpy).toHaveBeenCalledWith(
			'pipe-3',
			expect.objectContaining({ adm_status: 1 }),
			['http://test-cluster']
		)
	})

	it('menampilkan tombol Start Selected saat baris table dipilih', async () => {
		const pinia = createPinia()
		setActivePinia(pinia)
		const store = usePipelineStore()
		store.pipelines = JSON.parse(JSON.stringify(dummyPipelines))

		const wrapper = mount(PipelineTable, {
			global: {
				plugins: [pinia],
				stubs: commonStubs
			}
		})

		await flushPromises()

		const vm = wrapper.vm as any
		expect(wrapper.text()).not.toContain('Start Selected')

		vm.selectedPipelines = [dummyPipelines[0], dummyPipelines[1]]
		await flushPromises()

		expect(wrapper.text()).toContain('Start Selected')
		expect(wrapper.text()).toContain('Stop Selected')
		expect(wrapper.text()).toContain('Delete Selected')
	})

	it('membuka modal konfirmasi status saat aksi Stop atau Start dipicu', async () => {
		const pinia = createPinia()
		setActivePinia(pinia)
		const store = usePipelineStore()
		store.pipelines = JSON.parse(JSON.stringify(dummyPipelines))

		const wrapper = mount(PipelineTable, {
			global: {
				plugins: [pinia],
				stubs: commonStubs
			}
		})

		await flushPromises()

		const vm = wrapper.vm as any
		expect(vm.showModalStatus).toBe(false)

		vm.promptStatusSingle(dummyPipelines[0], 'stop')
		expect(vm.showModalStatus).toBe(true)
		expect(vm.statusModalTitle).toBe('Stop Pipeline')
		expect(vm.statusModalMessage).toContain('Network Telemetry Pipeline')

		vm.selectedPipelines = [dummyPipelines[1], dummyPipelines[2]]
		vm.promptStatusBulk('start')
		expect(vm.showModalStatus).toBe(true)
		expect(vm.statusModalTitle).toBe('Start Selected Pipelines')
		expect(vm.statusModalMessage).toContain('2 selected pipelines')
	})
})

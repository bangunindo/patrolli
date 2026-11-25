// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it, vi, beforeEach } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import ClusterTable from '@/components/cluster/Table.vue'
import { useClusterStore } from '@/stores/cluster.store'
import { formatUtcDate } from '@/app/helpers/date.helper'
import nodesApi from '@/app/api/nodes.api'
import type { IClusterNode } from '@@/shared/types/cluster.type'

vi.mock('vue-router', () => ({
	useRouter: () => ({
		push: vi.fn()
	})
}))

vi.mock('@/app/api/nodes.api', () => ({
	default: {
		getNodes: vi.fn()
	}
}))

describe('Cluster Table - Last Heartbeat formatting', () => {
	beforeEach(() => {
		setActivePinia(createPinia())
		vi.mocked(nodesApi.getNodes).mockResolvedValue({
			success: true,
			status_code: 200,
			data: { nodes: [] }
		} as any)
	})

	const mountTable = (nodes: IClusterNode[]) => {
		const store = useClusterStore()
		store.nodes = nodes

		return mount(ClusterTable, {
			global: {
				stubs: {
					AppTable: {
						props: ['columns', 'data', 'loading'],
						template: `
							<div class="mock-app-table">
								<div v-for="(row, index) in data" :key="index" class="mock-row">
									<span class="heartbeat-cell">
										<slot name="col.last_heartbeat" :data="row" />
									</span>
								</div>
							</div>
						`
					},
					UiButton: true,
					UiIcon: true
				}
			}
		})
	}

	it('renders last_heartbeat with date and time down to seconds', () => {
		const isoTimestamp = '2026-11-24T10:43:15Z'
		const expected = formatUtcDate(isoTimestamp)

		const wrapper = mountTable([
			{
				node_id: 'node-1',
				name: 'patrolli-node-1',
				host_count: 3,
				op_status: 'active',
				is_leader: true,
				metadata: { advertise_address: '10.0.0.1:8080' },
				last_heartbeat: isoTimestamp
			} as IClusterNode
		])

		const cellText = wrapper.find('.heartbeat-cell').text()
		expect(cellText).toBe(expected)
		expect(cellText).toMatch(/^\d{2} [A-Z][a-z]{2} \d{4} \d{2}:\d{2}:\d{2}$/)
	})

	it('renders fallback dash when last_heartbeat is invalid or empty', () => {
		const wrapper = mountTable([
			{
				node_id: 'node-2',
				name: 'patrolli-node-2',
				host_count: 0,
				op_status: 'inactive',
				is_leader: false,
				metadata: { advertise_address: '10.0.0.2:8080' },
				last_heartbeat: ''
			} as IClusterNode
		])

		expect(wrapper.find('.heartbeat-cell').text()).toBe('-')
	})
})

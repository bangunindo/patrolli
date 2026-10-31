// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import nodesApi from '@/app/api/nodes.api'
import type { IClusterNode } from '@@/shared/types/cluster.type'
import { defineStore } from 'pinia'
import { reactive, ref } from 'vue'

export const useClusterStore = defineStore(
	'cluster',
	() => {
		const nodes = ref<IClusterNode[]>([])
		const loading = reactive({
			table: false
		})
		const error = ref<string | null>(null)

		const fetchNodes = async () => {
			loading.table = true
			error.value = null
			try {
				const response = await nodesApi.getNodes()
				if (response?.success && response.data?.nodes) {
					nodes.value = response.data.nodes
				}
			} catch (err: any) {
				error.value = err.message || 'Failed to fetch nodes'
			} finally {
				loading.table = false
			}
		}

		const modalShow = ref<boolean>(false)
		const toggleModalForm = (value: boolean) => {
			modalShow.value = value
		}

		return {
			nodes,
			loading,
			error,
			fetchNodes,
			modalShow,
			toggleModalForm
		}
	},
	{ persist: false }
)

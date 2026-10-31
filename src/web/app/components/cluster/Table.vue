<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import AppTable from '@/components/app/AppTable.vue'
import { useClusterStore } from '@/stores/cluster.store'
import { onMounted } from 'vue'
import { useRouter } from 'vue-router'

const router = useRouter()
const clusterStore = useClusterStore()

const headerTable = [
	{
		key: 'name',
		title: 'Node Name'
	},
	{
		key: 'host_count',
		title: 'Host Count'
	},
	{
		key: 'op_status',
		title: 'Status',
		align: 'center' as const
	},
	{
		key: 'is_leader',
		title: 'Leader',
		align: 'center' as const
	},
	{
		key: 'advertise_address',
		title: 'Address',
		align: 'right' as const
	},
	{
		key: 'last_heartbeat',
		title: 'Last Heartbeat',
		align: 'right' as const
	}
]
const emits = defineEmits(['navtoHost'])

const navigateToHosts = (data) => {
	router.push(`/configuration/host?monitored_by=${data.node_id}`)
	emits('navtoHost')
}

onMounted(() => {
	clusterStore.fetchNodes()
})

const getStatusColor = (status: string) => {
	switch (status) {
		case 'active':
			return 'bg-emerald-500'
		case 'inactive':
			return 'bg-rose-500'
		case 'initializing':
			return 'bg-amber-500'
		default:
			return 'bg-sky-500'
	}
}
</script>

<template>
	<div class="w-full mt-4">
		<AppTable
			v-bind="{
				columns: headerTable,
				data: clusterStore.nodes,
				loading: clusterStore.loading.table
			}"
			:use-selection="false"
			:use-action="false"
		>
			<template #col.op_status="prop">
				<div class="flex flex-row flex-wrap gap-2 items-center justify-center">
					<div
						class="w-2.5 h-2.5 rounded-full shrink-0"
						:class="getStatusColor(prop.data.op_status)"
					></div>
					<div class="text-sm capitalize">
						{{ prop.data.op_status }}
					</div>
				</div>
			</template>

			<template #col.host_count="prop">
				<UiButton
					variant="link"
					size="sm"
					class="h-auto p-0 text-primary gap-1 font-normal"
					@click="navigateToHosts(prop.data)"
				>
					<UiIcon name="format-list-bulleted" size="14px" />
					<span>{{ prop.data.host_count ?? 0 }} Host</span>
				</UiButton>
			</template>

			<template #col.is_leader="prop">
				{{ prop.data.is_leader ? 'Yes' : 'No' }}
			</template>

			<template #col.advertise_address="prop">
				{{ prop.data.metadata.advertise_address }}
			</template>

			<template #col.last_heartbeat="prop">
				{{ new Date(prop.data.last_heartbeat).toDateString() }}
			</template>
		</AppTable>
	</div>
</template>

<style scoped>
:deep(th > div.flex.flex-row.items-center > div) {
	width: unset;
}
</style>

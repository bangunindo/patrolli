<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import ClusterTable from '@/components/cluster/Table.vue'
import { useClusterStore } from '@/stores/cluster.store'

definePageMeta({
	layout: 'default',
	requiresAuth: true
})

const clusterStore = useClusterStore()
</script>

<template>
	<div class="p-4 md:p-6 w-full">
		<AppLayoutPageContainer>
			<AppLayoutPageHeader
				title="Clusters"
				description="Overview of all registered nodes in the cluster."
			>
				<template #actions>
					<UiButton
						variant="outline"
						class="gap-1.5 shadow-xs"
						:disabled="clusterStore.loading.table"
						@click="clusterStore.fetchNodes()"
					>
						<UiIcon
							name="refresh"
							size="16px"
							:class="{ 'animate-spin': clusterStore.loading.table }"
						/>
						<span>Refresh</span>
					</UiButton>
				</template>
			</AppLayoutPageHeader>

			<cluster-table />
		</AppLayoutPageContainer>
	</div>
</template>

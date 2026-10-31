<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import ModalForm from '@/components/pipeline/ModalForm.vue'
import PipelineTable from '@/components/pipeline/Table.vue'
import { usePipelineStore } from '@/stores/pipeline.store'

definePageMeta({
	layout: 'default',
	requiresAuth: true
})

const pipelineStore = usePipelineStore()
</script>

<template>
	<div class="p-4 md:p-6 w-full">
		<AppLayoutPageContainer>
			<AppLayoutPageHeader
				title="Pipeline Management"
				description="Create, configure, and monitor independent telemetry data pipelines."
			>
				<template #actions>
					<UiButton @click="pipelineStore.togglePipelineModalForm(true)">
						<UiIcon name="plus" size="16px" />
						<span>Create Pipeline</span>
					</UiButton>
				</template>
			</AppLayoutPageHeader>

			<pipeline-table />
		</AppLayoutPageContainer>

		<modal-form v-if="pipelineStore.pipelineModalShow" />
	</div>
</template>

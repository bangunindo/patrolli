<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import PipelineForm from '@/components/pipeline/Form.vue'
import { usePipelineStore } from '@/stores/pipeline.store'

const pipelineStore = usePipelineStore()
</script>

<template>
	<UiDialog v-model:open="pipelineStore.pipelineModalShow">
		<UiDialogContent class="sm:max-w-4xl p-0 overflow-hidden border-none shadow-2xl">
			<PipelineForm />
		</UiDialogContent>
	</UiDialog>
</template>

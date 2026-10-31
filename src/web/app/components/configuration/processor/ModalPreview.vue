<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { usePipelineBuildStore } from '@/stores/pipeline.builder.store'
import { watch } from 'vue'
import ProcessorPreview from '@/components/configuration/processor/Preview.vue'

const modelValue = defineModel({ default: false })

const pipelineBuilderStore = usePipelineBuildStore()

watch(
	() => modelValue.value,
	(val) => {
		if (!val) {
			pipelineBuilderStore.updateCanvas()
		}
	}
)
</script>

<template>
	<UiDialog v-model:open="modelValue">
		<UiDialogContent class="max-w-[95vw] w-[95vw] max-h-[90vh] p-6 flex flex-col">
			<ProcessorPreview v-if="modelValue" />
		</UiDialogContent>
	</UiDialog>
</template>

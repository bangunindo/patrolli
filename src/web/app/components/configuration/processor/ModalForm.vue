<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { useProcessorStore } from '@/stores/processor.store'
import { storeToRefs } from 'pinia'

import ProcessorForm from '@/components/configuration/processor/Form.vue'

const processorStore = useProcessorStore()

const { processorModalShow } = storeToRefs(processorStore)
</script>

<template>
	<UiDialog v-model:open="processorModalShow">
		<UiDialogContent class="max-w-2xl max-h-[85vh] flex flex-col p-6">
			<ProcessorForm />
		</UiDialogContent>
	</UiDialog>
</template>

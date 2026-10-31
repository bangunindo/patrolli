<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { useSinksStore } from '@/stores/sinks.store'
import { storeToRefs } from 'pinia'

import SinkForm from '@/components/configuration/sinks/Form.vue'

const sinksStore = useSinksStore()

const { sinkModalShow } = storeToRefs(sinksStore)
</script>

<template>
	<UiDialog v-model:open="sinkModalShow">
		<UiDialogContent class="max-w-2xl max-h-[85vh] flex flex-col p-6">
			<SinkForm />
		</UiDialogContent>
	</UiDialog>
</template>

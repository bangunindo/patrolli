<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { useListenerStore } from '@/stores/listeners.store'
import { storeToRefs } from 'pinia'

import ListenerForm from '@/components/configuration/listener/Form.vue'

const listenerStore = useListenerStore()

const { listenerModalShow } = storeToRefs(listenerStore)
</script>

<template>
	<UiDialog v-model:open="listenerModalShow">
		<UiDialogContent class="max-w-2xl max-h-[85vh] flex flex-col p-6">
			<ListenerForm />
		</UiDialogContent>
	</UiDialog>
</template>

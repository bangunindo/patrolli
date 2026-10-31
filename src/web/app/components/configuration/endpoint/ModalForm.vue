<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { useEndpointsStore } from '@/stores/endpoints.store'
import { storeToRefs } from 'pinia'

import EndpointForm from '@/components/configuration/endpoint/Form.vue'

const endpointsStore = useEndpointsStore()

const { endpointModalShow } = storeToRefs(endpointsStore)
</script>

<template>
	<UiDialog v-model:open="endpointModalShow">
		<UiDialogContent class="max-w-2xl max-h-[85vh] flex flex-col p-6">
			<EndpointForm />
		</UiDialogContent>
	</UiDialog>
</template>

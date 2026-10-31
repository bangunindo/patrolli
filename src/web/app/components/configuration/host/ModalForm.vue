<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { useHostsStore } from '@/stores/hosts.store'
import { storeToRefs } from 'pinia'

import HostForm from '@/components/configuration/host/Form.vue'

const hostsStore = useHostsStore()

const { hostModalShow } = storeToRefs(hostsStore)
</script>

<template>
	<UiDialog v-model:open="hostModalShow">
		<UiDialogContent class="max-w-2xl max-h-[85vh] flex flex-col p-6">
			<HostForm />
		</UiDialogContent>
	</UiDialog>
</template>

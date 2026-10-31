<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { storeToRefs } from 'pinia'

import { useClusterStore } from '@/stores/cluster.store'
import Table from '@/components/cluster/Table.vue'

const store = useClusterStore()

const { modalShow } = storeToRefs(store)
</script>

<template>
	<UiDialog v-model:open="modalShow">
		<UiDialogContent class="sm:max-w-5xl">
			<UiDialogHeader>
				<div class="flex flex-row gap-3 items-center">
					<div
						class="flex flex-row justify-center items-center p-2 rounded-lg bg-sky-100 dark:bg-sky-950 text-sky-600 dark:text-sky-400"
					>
						<UiIcon name="server" size="20px" />
					</div>
					<UiDialogTitle class="text-base font-semibold"> Cluster Nodes </UiDialogTitle>
				</div>
			</UiDialogHeader>

			<div class="py-2 overflow-auto max-h-[75vh]">
				<Table v-if="modalShow" @navto-host="modalShow = false" />
			</div>
		</UiDialogContent>
	</UiDialog>
</template>

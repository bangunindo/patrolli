<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { onMounted, ref } from 'vue'

import { useSinksStore } from '@/stores/sinks.store'

import AppTable from '@/components/app/AppTable.vue'
import ModalDetail from '@/components/configuration/sinks/ModalDetail.vue'
import ModalSinkForm from '@/components/configuration/sinks/ModalForm.vue'

import ModalConfirmationDelete from '@/components/modal/ModalConfirmationDelete.vue'
import { FormMode } from '@@/shared/types/app.type'
import type { ISink } from '@@/shared/types/sink.type'
import { watchDebounced } from '@vueuse/core'

const sinksStore = useSinksStore()

const showModalDelete = ref<boolean>(false)
const showModalDetail = ref<boolean>(false)

const headerTable = [
	{
		key: 'name',
		title: 'Name',
		sortable: true
	},
	{
		key: 'adm_status',
		title: 'Status',
		sortable: true
	},
	{
		key: 'labels',
		title: 'Labels'
	},
	{
		key: 'type',
		title: 'Type'
	},
	{
		key: 'batch_size',
		title: 'Batch Size'
	},
	{
		key: 'batch_timeout',
		title: 'Batch Timeout'
	},
	{
		key: 'channel_compression',
		title: 'Channel Compression'
	},
	{
		key: 'channel_type',
		title: 'Channel Type'
	}
]

function onChangePage(payload) {
	if (sinksStore.filter.offset != payload.offset) {
		sinksStore.filter.limit = payload.limit
		sinksStore.filter.offset = payload.offset
		sinksStore.fetchSinks()
	}
}

function parsedLabelsData(labels: any) {
	if (!labels) return {}
	if (typeof labels === 'object') return labels
	try {
		return JSON.parse(labels) || {}
	} catch {
		return {}
	}
}

function toggleModalDelete(sink: ISink | null = null) {
	sinksStore.currentSink = sink
	showModalDelete.value = !showModalDelete.value
}

function toggleModalDetail(sink: ISink | null = null) {
	sinksStore.currentSink = sink
	showModalDetail.value = !showModalDetail.value
}

async function onDelete() {
	const clusterUrl = sinksStore.currentSink?.cluster_url
	await sinksStore.deleteSink(
		sinksStore.currentSink?.sink_id ?? '',
		clusterUrl ? [clusterUrl] : undefined
	)
	showModalDelete.value = false
	sinksStore.fetchSinks()
}

onMounted(() => {
	sinksStore.filter.offset = 0
	sinksStore.fetchSinks()
})

// search
watchDebounced(
	() => sinksStore.filter.name,
	async () => {
		sinksStore.filter.offset = 0
		sinksStore.fetchSinks()
	},
	{
		debounce: 500
	}
)
</script>

<template>
	<div class="w-full space-y-4">
		<div class="flex flex-col sm:flex-row items-center justify-between gap-3 w-full">
			<div class="relative w-full sm:w-80">
				<UiIcon
					name="magnify"
					size="18px"
					class="absolute left-3 top-1/2 -translate-y-1/2 text-muted-foreground pointer-events-none"
				/>
				<UiInput
					v-model="sinksStore.filter.name"
					placeholder="Search by name..."
					class="pl-9 h-9 border-[#D8D9DB] dark:border-border rounded-md shadow-xs text-sm"
				/>
			</div>
		</div>

		<AppTable
			:columns="headerTable"
			:data="sinksStore.sinks"
			:loading="sinksStore.loading.table"
			:pagination="sinksStore.pagination"
			@update:page="onChangePage"
		>
			<template #col.name="prop">
				<div class="font-semibold">{{ prop.data.name }}</div>
				<span class="text-xs text-muted-foreground">{{ prop.data.cluster_name }}</span>
			</template>

			<template #col.adm_status="prop">
				<div class="flex flex-row flex-wrap gap-2 items-center">
					<div
						class="w-2.5 h-2.5 rounded-full"
						:class="
							Number(prop.data.adm_status) === 1 ? 'bg-emerald-500' : 'bg-rose-500'
						"
					></div>
					<div class="text-sm">
						{{ Number(prop.data.adm_status) === 1 ? 'Active' : 'Inactive' }}
					</div>
				</div>
			</template>

			<template #col.labels="prop">
				<div
					class="flex flex-row flex-wrap gap-1"
					v-if="Object.keys(parsedLabelsData(prop.data.labels)).length > 0"
				>
					<UiBadge
						v-for="(value, key) in parsedLabelsData(prop.data.labels)"
						:key="`labels-${key}-${value}`"
						variant="outline"
						class="text-xs font-normal"
					>
						{{ key }} : {{ value }}
					</UiBadge>
				</div>
				<span v-else class="text-muted-foreground">-</span>
			</template>

			<template #col.type="prop">
				<UiBadge
					v-if="prop.data.config?.type"
					variant="secondary"
					class="font-normal text-xs uppercase tracking-wide"
				>
					{{ prop.data.config.type }}
				</UiBadge>
				<span v-else class="text-muted-foreground">-</span>
			</template>

			<template #col.batch_size="prop">
				{{ prop.data.batch.batch_size }}
			</template>
			<template #col.batch_timeout="prop">
				{{ prop.data.batch.batch_timeout }}
			</template>
			<template #col.channel_compression="prop">
				{{ prop.data.channel.compression }}
			</template>
			<template #col.channel_type="prop">
				{{ prop.data.channel.type }}
			</template>

			<template #col.action="prop">
				<div class="flex flex-row justify-end">
					<UiDropdownMenu>
						<UiDropdownMenuTrigger as-child>
							<UiButton
								variant="ghost"
								size="icon"
								class="h-8 w-8 text-muted-foreground hover:text-foreground"
							>
								<UiIcon name="dots-vertical" size="18px" />
							</UiButton>
						</UiDropdownMenuTrigger>

						<UiDropdownMenuContent align="end" class="w-44">
							<UiDropdownMenuItem
								class="cursor-pointer gap-2"
								@click="toggleModalDetail(prop.data)"
							>
								<UiIcon name="eye-outline" size="16px" />
								<span>View Detail</span>
							</UiDropdownMenuItem>

							<UiDropdownMenuItem
								class="cursor-pointer gap-2"
								@click="
									sinksStore.toggleSinkModalForm(true, FormMode.UPDATE, prop.data)
								"
							>
								<UiIcon name="pencil-outline" size="16px" />
								<span>Edit Sink</span>
							</UiDropdownMenuItem>

							<UiDropdownMenuSeparator />

							<UiDropdownMenuItem
								class="cursor-pointer gap-2 text-destructive focus:text-destructive focus:bg-destructive/10"
								@click="toggleModalDelete(prop.data)"
							>
								<UiIcon name="delete-outline" size="16px" />
								<span>Delete Sink</span>
							</UiDropdownMenuItem>
						</UiDropdownMenuContent>
					</UiDropdownMenu>
				</div>
			</template>
		</AppTable>
	</div>

	<ModalSinkForm v-if="sinksStore.sinkModalShow" />

	<ModalDetail
		v-if="showModalDetail"
		:show-modal="showModalDetail"
		:sink="sinksStore.currentSink ?? null"
		@dismiss="toggleModalDetail(null)"
	/>

	<ModalConfirmationDelete
		:show-modal="showModalDelete"
		@dismiss="showModalDelete = false"
		@confirm="onDelete"
	/>
</template>

<style scoped>
:deep(th > div.flex.flex-row.items-center > div) {
	width: unset;
	white-space: nowrap;
}
</style>

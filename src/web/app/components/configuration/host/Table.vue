<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { useHostsStore } from '@/stores/hosts.store'
import { FormMode } from '@@/shared/types/app.type'
import type { IHost } from '@@/shared/types/host.type'
import { watchDebounced } from '@vueuse/core'
import { onBeforeUnmount, onMounted, ref } from 'vue'
import { useRoute } from 'vue-router'

import AppTable from '@/components/app/AppTable.vue'
import AppInputSelect from '@/components/app/input/AppInputSelect.vue'
import ModalDetail from '@/components/configuration/host/ModalDetail.vue'
import ModalHostForm from '@/components/configuration/host/ModalForm.vue'
import ModalConfirmationDelete from '@/components/modal/ModalConfirmationDelete.vue'
import ToggleTemplate from '@/components/configuration/host/ToggleTemplate.vue'

const route = useRoute()
const hostsStore = useHostsStore()

const showModalDelete = ref<boolean>(false)
const showModalDetail = ref<boolean>(false)
const selectedHosts = ref<IHost[]>([])

function toggleModalDetail(host: IHost | null = null) {
	hostsStore.currentHost = host
	showModalDetail.value = !showModalDetail.value
}

const headerTable = [
	{
		key: 'name',
		title: 'Host Name',
		sortable: true
	},
	{
		key: 'adm_status',
		title: 'Status'
	},
	{
		key: 'labels',
		title: 'Labels'
	},
	{
		key: 'templates',
		title: 'Templates'
	},
	{
		key: 'item_count',
		title: 'Item Count'
	},
	{
		key: 'monitored_by_node_name',
		title: 'Cluster Node'
	}
]

function parsedLabelsData(labels: string) {
	if (!labels) return {}
	try {
		return JSON.parse(labels) || {}
	} catch (e) {
		return {}
	}
}

function onChangePage(payload) {
	if (hostsStore.filter.offset != payload.offset) {
		hostsStore.filter.limit = payload.limit
		hostsStore.filter.offset = payload.offset
		hostsStore.fetchHosts()
	}
}

function onSort(payload) {
	hostsStore.filter.order_by = payload.key ?? undefined
	hostsStore.filter.order_dir = payload.order ?? undefined

	hostsStore.fetchHosts()
}

function toggleModalDelete(host: IHost | null = null) {
	hostsStore.currentHost = host
	showModalDelete.value = !showModalDelete.value
}

async function onDelete() {
	const clusterUrl = hostsStore.currentHost?.cluster_url
	await hostsStore.deleteHost(
		hostsStore.currentHost?.host_id ?? '',
		clusterUrl ? [clusterUrl] : undefined
	)
	showModalDelete.value = false
	hostsStore.fetchHosts()
}

async function onFilter() {
	hostsStore.filter.offset = 0
	hostsStore.fetchHosts()
}

onMounted(() => {
	if (route.query.monitored_by) {
		hostsStore.filter.offset = 0
		hostsStore.filter.monitored_by = route.query.monitored_by as string
	} else {
		hostsStore.filter.offset = 0
	}

	hostsStore.fetchHosts()
	hostsStore.fetchNodes()
})

onBeforeUnmount(() => {
	hostsStore.filter.monitored_by = undefined
})

// search
watchDebounced(
	() => hostsStore.filter.name,
	async () => {
		onFilter()
	},
	{
		debounce: 500
	}
)

hostsStore.$onAction((context) => {
	if (context.name == 'fetchHosts') {
		context.after(() => {
			const selected = hostsStore.hosts.filter((host) => {
				const hostIds = selectedHosts.value.flatMap((h: any) => h.host_id)
				return host.host_id.includeTexts(hostIds)
			})

			selectedHosts.value = selected
		})
	}
})
</script>

<template>
	<div class="w-full space-y-4">
		<div class="flex flex-col sm:flex-row items-center justify-between gap-3 w-full">
			<div class="flex flex-col sm:flex-row items-center gap-3 w-full sm:w-auto flex-1">
				<div class="relative w-full sm:w-80">
					<UiIcon
						name="magnify"
						size="18px"
						class="absolute left-3 top-1/2 -translate-y-1/2 text-muted-foreground pointer-events-none"
					/>
					<UiInput
						v-model="hostsStore.filter.name"
						placeholder="Search by name..."
						class="pl-9 h-9 border-[#D8D9DB] dark:border-border rounded-md shadow-xs text-sm"
					/>
				</div>
				<div class="w-full sm:w-56">
					<AppInputSelect
						v-model="hostsStore.filter.monitored_by"
						variant="outline"
						placeholder="Select Node"
						:options="hostsStore.selection.nodes"
						clearable
						@update:model-value="onFilter"
					/>
				</div>
			</div>
		</div>

		<div class="w-full space-y-3">
			<ToggleTemplate v-if="selectedHosts.length > 0" :hosts="selectedHosts" />

			<AppTable
				v-bind="{
					columns: headerTable,
					data: hostsStore.hosts,
					loading: hostsStore.loading.table,
					pagination: hostsStore.pagination
				}"
				v-model:selected="selectedHosts"
				selection-key="host_id"
				use-selection
				@update:sort="onSort"
				@update:page="onChangePage"
			>
				<template #col.name="prop">
					<div class="font-semibold text-foreground">{{ prop.data.name }}</div>
					<span class="text-xs text-muted-foreground">{{ prop.data.cluster_name }}</span>
				</template>

				<template #col.adm_status="prop">
					<div class="flex flex-row flex-wrap gap-2 items-center">
						<div
							class="w-2 h-2 rounded-full"
							:class="prop.data.adm_status === 1 ? 'bg-emerald-500' : 'bg-rose-500'"
						></div>
						<div class="text-sm font-medium">
							{{ prop.data.adm_status === 1 ? 'Active' : 'Inactive' }}
						</div>
					</div>
				</template>

				<template #col.labels="prop">
					<div
						class="flex flex-row flex-wrap gap-1.5"
						v-if="Object.keys(parsedLabelsData(prop.data.labels)).length > 0"
					>
						<span
							v-for="(value, key) in parsedLabelsData(prop.data.labels)"
							:key="`labels-${key}-${value}`"
							class="inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium bg-[#E0F2FE] dark:bg-sky-950/60 text-[#0284C7] dark:text-sky-400 border border-[#BAE6FD]/60 dark:border-sky-800/60"
						>
							{{ key }} : {{ value }}
						</span>
					</div>
					<span v-else class="text-muted-foreground">-</span>
				</template>

				<template #col.templates="prop">
					<div class="flex flex-row flex-wrap gap-1" v-if="prop.data.templates?.length">
						<UiBadge
							v-for="template in prop.data.templates"
							:key="template.template_id"
							variant="secondary"
							class="text-xs font-normal"
						>
							{{ template.name || template.template_id }}
						</UiBadge>
					</div>
					<span v-else class="text-muted-foreground">-</span>
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
										hostsStore.toggleHostModalForm(
											true,
											FormMode.UPDATE,
											prop.data
										)
									"
								>
									<UiIcon name="pencil-outline" size="16px" />
									<span>Edit Host</span>
								</UiDropdownMenuItem>

								<UiDropdownMenuSeparator />

								<UiDropdownMenuItem
									class="cursor-pointer gap-2 text-destructive focus:text-destructive focus:bg-destructive/10"
									@click="toggleModalDelete(prop.data)"
								>
									<UiIcon name="delete-outline" size="16px" />
									<span>Delete Host</span>
								</UiDropdownMenuItem>
							</UiDropdownMenuContent>
						</UiDropdownMenu>
					</div>
				</template>
			</AppTable>
		</div>
	</div>

	<ModalHostForm v-if="hostsStore.hostModalShow" />

	<ModalDetail
		:show-modal="showModalDetail"
		:host="hostsStore.currentHost"
		@dismiss="showModalDetail = false"
	/>

	<ModalConfirmationDelete
		title="Delete Host"
		:show-modal="showModalDelete"
		:message="`Are you sure you want to delete this host?\n\nHost: ${hostsStore.currentHost?.name}\n\nThis action cannot be undone.`"
		@dismiss="showModalDelete = false"
		@confirm="onDelete"
	/>
</template>

<style scoped>
:deep(th > div.flex.flex-row.items-center > div) {
	width: unset;
}
</style>

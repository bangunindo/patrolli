<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'
import { watchDebounced } from '@vueuse/core'
import AppTable from '@/components/app/AppTable.vue'
import ModalConfirmationDelete from '@/components/modal/ModalConfirmationDelete.vue'
import { usePipelineStore } from '@/stores/pipeline.store'
import { FormMode } from '@@/shared/types/app.type'
import type { IPipelineItem } from '@@/shared/types/pipeline.management.type'

const router = useRouter()
const pipelineStore = usePipelineStore()

// Selection state for AppTable
const selectedPipelines = ref<IPipelineItem[]>([])

// Delete confirmation modal state
const showModalDelete = ref(false)
const pendingDeleteType = ref<'single' | 'bulk'>('single')
const pendingSinglePipeline = ref<IPipelineItem | null>(null)

// Status confirmation modal state (Start / Stop)
const showModalStatus = ref(false)
const pendingStatusAction = ref<'start' | 'stop'>('start')
const pendingStatusType = ref<'single' | 'bulk'>('single')
const pendingSingleStatusPipeline = ref<IPipelineItem | null>(null)

function promptStatusSingle(pipeline: IPipelineItem, action: 'start' | 'stop') {
	pendingStatusAction.value = action
	pendingStatusType.value = 'single'
	pendingSingleStatusPipeline.value = pipeline
	showModalStatus.value = true
}

function promptStatusBulk(action: 'start' | 'stop') {
	pendingStatusAction.value = action
	pendingStatusType.value = 'bulk'
	showModalStatus.value = true
}

const statusModalTitle = computed(() => {
	const actionText = pendingStatusAction.value === 'start' ? 'Start' : 'Stop'
	if (pendingStatusType.value === 'single') {
		return `${actionText} Pipeline`
	}
	return `${actionText} Selected Pipelines`
})

const statusModalMessage = computed(() => {
	const actionText = pendingStatusAction.value === 'start' ? 'start' : 'stop'
	if (pendingStatusType.value === 'single') {
		return `Are you sure you want to ${actionText} the pipeline "${pendingSingleStatusPipeline.value?.name}"?`
	}
	return `Are you sure you want to ${actionText} ${selectedPipelines.value.length} selected pipelines?`
})

async function onStatusConfirm() {
	if (pendingStatusType.value === 'single' && pendingSingleStatusPipeline.value) {
		await pipelineStore.togglePipelineStatus(
			pendingSingleStatusPipeline.value.id,
			pendingSingleStatusPipeline.value.cluster_url
		)
	} else if (pendingStatusType.value === 'bulk') {
		const ids = selectedPipelines.value.map((p) => p.id)
		if (pendingStatusAction.value === 'start') {
			await pipelineStore.startSelected(ids)
		} else {
			await pipelineStore.stopSelected(ids)
		}
		selectedPipelines.value = []
	}
	showModalStatus.value = false
}

const headerTable = [
	{
		key: 'name',
		title: 'Pipeline Name',
		sortable: true
	},
	{
		key: 'status',
		title: 'Status',
		sortable: true
	},
	{
		key: 'access',
		title: 'Access'
	},
	{
		key: 'assigned_resources',
		title: 'Assigned Resources'
	},
	{
		key: 'last_update',
		title: 'Last Update'
	}
]

function onChangePage(payload: { limit: number; offset: number; page?: number }) {
	if (pipelineStore.filter.offset != payload.offset) {
		pipelineStore.filter.limit = payload.limit
		pipelineStore.filter.offset = payload.offset
		pipelineStore.fetchPipelines()
	}
}

function onSort(payload: { key?: string; order?: 'asc' | 'desc' | null }) {
	pipelineStore.filter.order_by = (payload.key as any) ?? undefined
	pipelineStore.filter.order_dir = payload.order ?? undefined

	pipelineStore.fetchPipelines()
}

async function onFilter() {
	pipelineStore.filter.offset = 0
	pipelineStore.fetchPipelines()
}

onMounted(() => {
	pipelineStore.filter.offset = 0
	pipelineStore.fetchPipelines()
})

watchDebounced(
	() => pipelineStore.filter.name,
	async () => {
		onFilter()
	},
	{
		debounce: 500
	}
)

pipelineStore.$onAction((context) => {
	if (context.name == 'fetchPipelines') {
		context.after(() => {
			const pipelineIds = selectedPipelines.value.flatMap((p: any) => p.id)
			const selected = pipelineStore.pipelines.filter((pipeline) => {
				return typeof pipeline.id.includeTexts === 'function'
					? pipeline.id.includeTexts(pipelineIds)
					: pipelineIds.includes(pipeline.id)
			})

			selectedPipelines.value = selected
		})
	}
})

async function navigateToBuilder(pipeline?: IPipelineItem) {
	if (!pipeline?.id) {
		router.push('/pipeline')
		return
	}

	const cUrl = pipeline.cluster_url || ''
	try {
		const detail = await pipelineStore.openPipelineDetail(pipeline.id, cUrl)
		if (detail?.canvases && detail.canvases.length > 0) {
			const sortedCanvases = [...detail.canvases].sort((a, b) => {
				return new Date(b.last_updated).getTime() - new Date(a.last_updated).getTime()
			})
			const latestCanvasId = sortedCanvases[0]?.canvas_id
			if (latestCanvasId) {
				router.push({
					path: `/pipeline/${pipeline.id}/builder/${latestCanvasId}`,
					query: cUrl ? { cluster_url: cUrl } : undefined
				})
				return
			}
		}
	} catch (err) {
		console.error('Failed to resolve latest canvas for pipeline:', err)
	}

	// Fallback if no canvas returned
	router.push({
		path: `/pipeline/${pipeline.id}/builder/default`,
		query: cUrl ? { cluster_url: cUrl } : undefined
	})
}

// Bulk Actions
function promptBulkDelete() {
	pendingDeleteType.value = 'bulk'
	showModalDelete.value = true
}

function promptDeleteSingle(pipeline: IPipelineItem) {
	pendingDeleteType.value = 'single'
	pendingSinglePipeline.value = pipeline
	showModalDelete.value = true
}

const deleteModalMessage = computed(() => {
	if (pendingDeleteType.value === 'single') {
		return `Are you sure you want to delete the pipeline "${pendingSinglePipeline.value?.name}"?\n\nThis action cannot be undone.`
	}
	return `Are you sure you want to delete ${selectedPipelines.value.length} selected pipelines?\n\nThis action cannot be undone.`
})

async function onDeleteConfirm() {
	if (pendingDeleteType.value === 'single' && pendingSinglePipeline.value) {
		await pipelineStore.deletePipeline(
			pendingSinglePipeline.value.id,
			pendingSinglePipeline.value.cluster_url
		)
		selectedPipelines.value = selectedPipelines.value.filter(
			(p) => p.id !== pendingSinglePipeline.value?.id
		)
	} else {
		const ids = selectedPipelines.value.map((p) => p.id)
		await pipelineStore.deleteSelected(ids)
		selectedPipelines.value = []
	}
	showModalDelete.value = false
}
</script>

<template>
	<div class="flex flex-col gap-4 w-full">
		<!-- Toolbar: Search & Bulk Actions -->
		<div class="flex flex-col md:flex-row md:items-center justify-between gap-3 w-full">
			<div class="flex flex-row items-center gap-2 w-full md:w-auto">
				<!-- Search -->
				<div class="relative w-full sm:w-80">
					<UiIcon
						name="magnify"
						size="18px"
						class="absolute left-3 top-1/2 -translate-y-1/2 text-muted-foreground pointer-events-none"
					/>
					<UiInput
						v-model="pipelineStore.filter.name"
						placeholder="Search pipeline by name..."
						class="pl-9 h-9 border-[#D8D9DB] dark:border-border rounded-md shadow-xs text-sm"
					/>
				</div>
			</div>

			<!-- Bulk Actions Toolbar (Figma Design) -->
			<div v-if="selectedPipelines.length > 0" class="w-full md:w-auto">
				<div
					class="flex flex-wrap items-center gap-2 p-1 px-2 rounded-lg bg-sky-50 dark:bg-sky-950/40 border border-[#BAE6FD] dark:border-sky-800/80 animate-in fade-in duration-150"
				>
					<span
						class="px-2.5 py-1 rounded-md text-xs font-semibold bg-primary text-primary-foreground shadow-xs"
					>
						{{ selectedPipelines.length }} Selected
					</span>

					<!-- Stop Selected Button -->
					<UiButton
						variant="outline"
						size="sm"
						class="h-8 gap-1.5 text-xs text-foreground border-[#D8D9DB] dark:border-border bg-white dark:bg-card hover:bg-slate-50 dark:hover:bg-accent"
						@click="promptStatusBulk('stop')"
					>
						<UiIcon name="stop-circle-outline" size="14px" class="text-amber-500" />
						<span>Stop Selected</span>
					</UiButton>

					<!-- Start Selected Button -->
					<UiButton
						variant="outline"
						size="sm"
						class="h-8 gap-1.5 text-xs text-emerald-600 border-emerald-600/30 bg-white dark:bg-card hover:bg-emerald-50 dark:hover:bg-emerald-950/40"
						@click="promptStatusBulk('start')"
					>
						<UiIcon name="play-circle-outline" size="14px" />
						<span>Start Selected</span>
					</UiButton>

					<!-- Delete Selected Button -->
					<UiButton
						variant="outline"
						size="sm"
						class="h-8 gap-1.5 text-xs text-[#C5192D] border-[#C5192D]/40 bg-white dark:bg-card hover:bg-rose-50 dark:hover:bg-rose-950/40"
						@click="promptBulkDelete"
					>
						<UiIcon name="delete-outline" size="14px" />
						<span>Delete Selected</span>
					</UiButton>
				</div>
			</div>
		</div>

		<!-- Central AppTable Component -->
		<AppTable
			v-bind="{
				columns: headerTable,
				data: pipelineStore.pipelines,
				loading: pipelineStore.loading.table,
				pagination: pipelineStore.pagination
			}"
			v-model:selected="selectedPipelines"
			selection-key="id"
			use-selection
			use-action
			@update:sort="onSort"
			@update:page="onChangePage"
		>
			<!-- Col: Pipeline Name & Code -->
			<template #col.name="prop">
				<div class="flex flex-col gap-0.5">
					<div
						class="font-semibold text-sm cursor-pointer text-slate-900 dark:text-slate-100 hover:text-sky-600 dark:hover:text-sky-400 transition-colors"
						@click="navigateToBuilder(prop.data)"
					>
						{{ prop.data.name }}
					</div>
					<span class="text-xs text-slate-400 font-mono">{{ prop.data.code }}</span>
				</div>
			</template>

			<!-- Col: Status Badge (Figma Colors) -->
			<template #col.status="prop">
				<span
					v-if="prop.data.status === 'running'"
					class="inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium bg-[#EAF7EF] text-[#32B062] dark:bg-emerald-950/60 dark:text-emerald-400 border border-emerald-200/50 dark:border-emerald-800/50"
				>
					Running
				</span>
				<span
					v-else-if="prop.data.status === 'stopped'"
					class="inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium bg-[#F8E8E9] text-[#B91A24] dark:bg-rose-950/60 dark:text-rose-400 border border-rose-200/50 dark:border-rose-800/50"
				>
					Stopped
				</span>
				<span
					v-else
					class="inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium bg-[#FEF3C7] text-[#D97706] dark:bg-amber-950/60 dark:text-amber-400 border border-amber-200/50 dark:border-amber-800/50"
				>
					Draft
				</span>
			</template>

			<!-- Col: Access Badge (Figma Colors) -->
			<template #col.access="prop">
				<span
					v-if="prop.data.access === 'public'"
					class="inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium bg-accent text-accent-foreground border border-primary/20"
				>
					Public
				</span>
				<span
					v-else
					class="inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium bg-[#EEF2FF] text-[#4F46E5] dark:bg-indigo-950/60 dark:text-indigo-400 border border-indigo-200/60 dark:border-indigo-800/60"
				>
					Private
				</span>
			</template>

			<!-- Col: Assigned Resources (Figma Design) -->
			<template #col.assigned_resources="prop">
				<span class="text-xs font-medium text-slate-800 dark:text-slate-200">
					{{ prop.data.assignedHostsCount ?? prop.data.assignedHosts?.length ?? 0 }}
					Hosts ·
					{{ prop.data.assignedSinksCount ?? prop.data.assignedSinks?.length ?? 0 }}
					Sinks
				</span>
			</template>

			<!-- Col: Last Update -->
			<template #col.last_update="prop">
				<span class="text-xs font-medium text-slate-600 dark:text-slate-400">
					{{ prop.data.lastUpdate }}
				</span>
			</template>

			<!-- Col: Action Menu -->
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
								@click="
									pipelineStore.togglePipelineModalForm(
										true,
										FormMode.UPDATE,
										prop.data
									)
								"
							>
								<UiIcon name="pencil-outline" size="16px" />
								<span>Edit Pipeline</span>
							</UiDropdownMenuItem>
							<UiDropdownMenuItem
								class="cursor-pointer gap-2"
								@click="navigateToBuilder(prop.data)"
							>
								<UiIcon name="vector-polyline" size="16px" />
								<span>Open in Builder</span>
							</UiDropdownMenuItem>
							<UiDropdownMenuItem
								class="cursor-pointer gap-2"
								@click="
									promptStatusSingle(
										prop.data,
										prop.data.status === 'running' ? 'stop' : 'start'
									)
								"
							>
								<UiIcon
									:name="
										prop.data.status === 'running'
											? 'stop-circle-outline'
											: 'play-circle-outline'
									"
									size="16px"
								/>
								<span>{{
									prop.data.status === 'running'
										? 'Stop Pipeline'
										: 'Start Pipeline'
								}}</span>
							</UiDropdownMenuItem>
							<UiDropdownMenuSeparator />
							<UiDropdownMenuItem
								class="cursor-pointer gap-2 text-destructive focus:text-destructive focus:bg-destructive/10"
								@click="promptDeleteSingle(prop.data)"
							>
								<UiIcon name="delete-outline" size="16px" />
								<span>Delete Pipeline</span>
							</UiDropdownMenuItem>
						</UiDropdownMenuContent>
					</UiDropdownMenu>
				</div>
			</template>
		</AppTable>

		<!-- Confirmation Modal Status (Start / Stop Pipeline) -->
		<UiDialog v-model:open="showModalStatus">
			<UiDialogContent class="sm:max-w-md">
				<UiDialogHeader>
					<UiDialogTitle>{{ statusModalTitle }}</UiDialogTitle>
				</UiDialogHeader>

				<div class="py-4 text-center whitespace-pre-wrap text-sm text-muted-foreground">
					{{ statusModalMessage }}
				</div>

				<UiDialogFooter class="flex flex-row gap-2 justify-end">
					<UiButton variant="outline" size="sm" @click="showModalStatus = false">
						Cancel
					</UiButton>
					<UiButton
						size="sm"
						:variant="pendingStatusAction === 'start' ? 'default' : 'secondary'"
						:class="
							pendingStatusAction === 'stop'
								? 'bg-amber-600 hover:bg-amber-700 text-white'
								: ''
						"
						@click="onStatusConfirm"
					>
						{{ pendingStatusAction === 'start' ? 'Start' : 'Stop' }}
					</UiButton>
				</UiDialogFooter>
			</UiDialogContent>
		</UiDialog>

		<!-- Confirmation Modal Delete -->
		<ModalConfirmationDelete
			title="Delete Pipeline"
			:show-modal="showModalDelete"
			:message="deleteModalMessage"
			@dismiss="showModalDelete = false"
			@confirm="onDeleteConfirm"
		/>
	</div>
</template>

<style scoped>
:deep(th > div.flex.flex-row.items-center > div) {
	width: unset;
}
</style>

<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { onMounted, ref } from 'vue'

import { storeToRefs } from 'pinia'
import { useRouter } from 'vue-router'

import { useTemplatesStore } from '@/stores/templates.store'

import AppTable from '@/components/app/AppTable.vue'
import ModalDetail from '@/components/configuration/templates/ModalDetail.vue'
import ModalTemplateForm from '@/components/configuration/templates/ModalForm.vue'
import ModalConfirmationDelete from '@/components/modal/ModalConfirmationDelete.vue'
import { FormMode } from '@@/shared/types/app.type'
import type { ITemplate } from '@@/shared/types/template.type'
import { watchDebounced } from '@vueuse/core'

const router = useRouter()

const templatesStore = useTemplatesStore()

const { templates } = storeToRefs(templatesStore)

const showModalDelete = ref<boolean>(false)

const headerTable = [
	{
		key: 'name',
		title: 'Name',
		sortable: true
	},
	{
		key: 'item_count',
		title: 'Items'
	},
	{
		key: 'links_to_templates',
		title: 'Links To Template'
	},
	{
		key: 'labels',
		title: 'Labels'
	}
]

function navigateToItems(template: ITemplate) {
	router.push(
		`/configuration/template/${template.template_id}/items/?cluster=${template.cluster_name}`
	)
}

const showModalDetail = ref<boolean>(false)

function parsedLabelsData(labels: any) {
	if (!labels) return {}
	try {
		return typeof labels === 'string' ? JSON.parse(labels) : labels || {}
	} catch (e) {
		return {}
	}
}

function onChangePage(payload: { offset: number; limit: number }) {
	if (templatesStore.filter.offset != payload.offset) {
		templatesStore.filter.limit = payload.limit
		templatesStore.filter.offset = payload.offset
		templatesStore.fetchTemplates()
	}
}

function toggleModalDetail(template: ITemplate | null = null) {
	templatesStore.currentTemplate = template
	showModalDetail.value = !showModalDetail.value
}

async function openTemplateDetail(templateId: string, clusterUrl?: string) {
	showModalDetail.value = false
	const clusterURLs = clusterUrl ? [clusterUrl] : undefined
	const detail = await templatesStore.fetchTemplatesDetail(templateId, clusterURLs)
	if (detail) {
		templatesStore.currentTemplate = detail
		showModalDetail.value = true
	}
}

function toggleModalDelete(template: ITemplate | null = null) {
	templatesStore.currentTemplate = template
	showModalDelete.value = !showModalDelete.value
}

async function onDelete() {
	const clusterUrl = templatesStore.currentTemplate?.cluster_url
	await templatesStore.deleteTemplate(
		templatesStore.currentTemplate?.template_id ?? '',
		clusterUrl ? [clusterUrl] : undefined
	)
	showModalDelete.value = false
	templatesStore.fetchTemplates()
}

onMounted(() => {
	templatesStore.filter.offset = 0
	templatesStore.fetchTemplates()
})

watchDebounced(
	() => templatesStore.filter.name,
	async () => {
		templatesStore.filter.offset = 0
		templatesStore.fetchTemplates()
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
					v-model="templatesStore.filter.name"
					placeholder="Search by name..."
					class="pl-9 h-9 border-[#D8D9DB] dark:border-border rounded-md shadow-xs text-sm"
				/>
			</div>
		</div>

		<AppTable
			v-bind="{
				columns: headerTable,
				data: templates,
				loading: templatesStore.loading.table,
				pagination: templatesStore.pagination
			}"
			@update:page="onChangePage"
		>
			<template #col.name="prop">
				<div class="font-medium text-foreground">{{ prop.data.name }}</div>
				<span class="text-xs text-muted-foreground">{{ prop.data.cluster_name }}</span>
			</template>

			<template #col.labels="prop">
				<div
					v-if="Object.keys(parsedLabelsData(prop.data.labels)).length > 0"
					class="flex gap-1.5 flex-wrap max-w-sm"
				>
					<UiBadge
						v-for="(value, key) in parsedLabelsData(prop.data.labels)"
						:key="`labels-${key}-${value}`"
						variant="outline"
						class="text-xs font-normal"
					>
						{{ key }}: {{ value }}
					</UiBadge>
				</div>
				<span v-else class="text-muted-foreground">-</span>
			</template>

			<template #col.links_to_templates="prop">
				<div class="flex flex-row flex-wrap gap-1">
					<template
						v-if="
							prop.data.links_to_templates && prop.data.links_to_templates.length > 0
						"
					>
						<button
							v-for="link in prop.data.links_to_templates"
							:key="link.template_id"
							type="button"
							class="cursor-pointer"
							@click="openTemplateDetail(link.template_id)"
						>
							<UiBadge
								variant="secondary"
								class="hover:bg-accent text-xs font-normal"
							>
								{{ link.name }}
							</UiBadge>
						</button>
					</template>
					<span v-else class="text-muted-foreground">-</span>
				</div>
			</template>

			<template #col.item_count="prop">
				<UiButton
					variant="link"
					size="sm"
					class="h-auto p-0 text-primary gap-1 font-normal"
					@click="navigateToItems(prop.data)"
				>
					<UiIcon name="format-list-bulleted" size="14px" />
					<span>{{ prop.data.item_count ?? 0 }} Items</span>
				</UiButton>
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
									templatesStore.toggleTemplateModalForm(
										true,
										FormMode.UPDATE,
										prop.data
									)
								"
							>
								<UiIcon name="pencil-outline" size="16px" />
								<span>Edit Template</span>
							</UiDropdownMenuItem>
							<UiDropdownMenuSeparator />
							<UiDropdownMenuItem
								class="cursor-pointer gap-2 text-destructive focus:text-destructive focus:bg-destructive/10"
								@click="toggleModalDelete(prop.data)"
							>
								<UiIcon name="delete-outline" size="16px" />
								<span>Delete Template</span>
							</UiDropdownMenuItem>
						</UiDropdownMenuContent>
					</UiDropdownMenu>
				</div>
			</template>
		</AppTable>
	</div>

	<ModalTemplateForm v-if="templatesStore.templateModalShow" />

	<ModalDetail
		v-if="showModalDetail"
		:show-modal="showModalDetail"
		:template="templatesStore.currentTemplate ?? null"
		@dismiss="toggleModalDetail(null)"
		@open-template="openTemplateDetail"
	/>

	<ModalConfirmationDelete
		:show-modal="showModalDelete"
		:message="`Are you sure you want to delete this template?\n\nTemplate: ${templatesStore.currentTemplate?.name}\n\nThis action cannot be undone.`"
		@dismiss="showModalDelete = false"
		@confirm="onDelete"
		confirm-text="Delete"
	/>
</template>

<style scoped>
:deep(th > div.flex.flex-row.items-center > div) {
	width: unset;
}
</style>

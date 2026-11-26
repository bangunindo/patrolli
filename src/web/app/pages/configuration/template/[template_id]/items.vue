<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import appHelper from '@/app/helpers/app.helper'
import AppFilterDropdown from '@/components/app/AppFilterDropdown.vue'
import AppInputAutocomplete from '@/components/app/input/AppInputAutocomplete.vue'
import TemplateItemsTable from '@/components/configuration/template-items/Table.vue'
import { useTemplateItemsStore } from '@/stores/template-items.store'
import type { ItemType } from '@@/shared/types/item.type'
import { computed, onMounted, onUnmounted, reactive, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'

definePageMeta({
	layout: 'configuration',
	requiresAuth: true
})

const route = useRoute()
const router = useRouter()
const templateItemsStore = useTemplateItemsStore()

function goBack() {
	router.go(-1)
}

const getClusterUrls = () => appHelper.getClusterUrls(route.query.cluster as string)

const templateId = route.params.template_id as string
templateItemsStore.setTemplateId(templateId)

const localFilter = reactive<{
	types: ItemType[]
}>({
	types: templateItemsStore.filter.types ? [...templateItemsStore.filter.types] : []
})

onMounted(() => {
	templateItemsStore.fetchTemplate(getClusterUrls())
})

const hasActiveFilter = computed(() => localFilter.types.length > 0)

function resetFilter() {
	localFilter.types = []
}

function applyFilter() {
	templateItemsStore.filter.types =
		localFilter.types.length > 0 ? [...localFilter.types] : undefined
	templateItemsStore.pagination.currentPage = 1
	templateItemsStore.filter.offset = 0
	templateItemsStore.fetchItems(getClusterUrls())
}

watch(
	localFilter,
	() => {
		applyFilter()
	},
	{ deep: true }
)

onUnmounted(() => {
	templateItemsStore.resetStore()
})
</script>

<template>
	<AppLayoutPageContainer>
		<AppLayoutPageHeader title="Template Items" show-back @back="goBack">
			<template #description>
				<p class="text-sm text-muted-foreground">
					<template v-if="templateItemsStore.currentTemplate">
						Items for template:
						<span class="font-medium text-foreground">{{
							templateItemsStore.currentTemplate.name
						}}</span>
					</template>
					<template v-else-if="templateItemsStore.loading.template">
						Loading template...
					</template>
					<template v-else> Manage items in this template. </template>
				</p>
			</template>

			<template #actions>
				<AppFilterDropdown :active="hasActiveFilter" @reset="resetFilter">
					<AppInputAutocomplete
						v-model="localFilter.types"
						label="Type"
						placeholder="All Types"
						:options="templateItemsStore.selection.types"
						multiple
						:portal="false"
						clearable
					/>
				</AppFilterDropdown>

				<UiButton @click="templateItemsStore.toggleItemsModalForm(true)">
					<UiIcon name="plus" size="16px" />
					<span>Create Item for Template</span>
				</UiButton>
			</template>
		</AppLayoutPageHeader>

		<template-items-table />
	</AppLayoutPageContainer>
</template>

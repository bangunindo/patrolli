<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import appHelper from '@/app/helpers/app.helper'
import TemplateItemsTable from '@/components/configuration/template-items/Table.vue'
import { useTemplateItemsStore } from '@/stores/template-items.store'
import { onMounted, onUnmounted } from 'vue'
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

const templateId = route.params.template_id as string
templateItemsStore.setTemplateId(templateId)

onMounted(() => {
	const clusterName = route.query.cluster as string
	const clusterUrl = appHelper.getClusterValue(clusterName)
	const urls = clusterUrl ? [clusterUrl] : undefined

	templateItemsStore.fetchTemplate(urls)
})

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
				<UiButton @click="templateItemsStore.toggleItemsModalForm(true)">
					<UiIcon name="plus" size="16px" />
					<span>Create Item for Template</span>
				</UiButton>
			</template>
		</AppLayoutPageHeader>

		<template-items-table />
	</AppLayoutPageContainer>
</template>

<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import TemplateTable from '@/components/configuration/templates/Table.vue'
import { useTemplatesStore } from '@/stores/templates.store'

definePageMeta({
	layout: 'configuration',
	requiresAuth: true
})

const templatesStore = useTemplatesStore()
</script>

<template>
	<AppLayoutPageContainer>
		<AppLayoutPageHeader title="Templates">
			<template #actions>
				<UiButton @click="templatesStore.toggleTemplateModalForm(true)">
					<UiIcon name="plus" size="16px" />
					<span>Create Template</span>
				</UiButton>
			</template>
		</AppLayoutPageHeader>

		<template-table />
	</AppLayoutPageContainer>
</template>

<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import HostTable from '@/components/configuration/host/Table.vue'
import { useHostsStore } from '@/stores/hosts.store'

definePageMeta({
	layout: 'configuration',
	requiresAuth: true
})

const hostsStore = useHostsStore()
</script>

<template>
	<AppLayoutPageContainer>
		<AppLayoutPageHeader title="Host">
			<template #actions>
				<UiButton @click="hostsStore.toggleHostModalForm(true)">
					<UiIcon name="plus" size="16px" />
					<span>Create Host</span>
				</UiButton>
			</template>
		</AppLayoutPageHeader>

		<host-table />
	</AppLayoutPageContainer>
</template>

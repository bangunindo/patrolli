<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { onMounted } from 'vue'
import UserStatsCards from '@/components/users/UserStatsCards.vue'
import UserToolbar from '@/components/users/UserToolbar.vue'
import UserTable from '@/components/users/UserTable.vue'
import ModalUserForm from '@/components/users/ModalUserForm.vue'
import ModalDeleteUser from '@/components/users/ModalDeleteUser.vue'
import ModalUserDetail from '@/components/users/ModalUserDetail.vue'
import { useAppStore } from '@/stores/app.store'
import { useUsersStore } from '@/stores/users.store'

definePageMeta({
	layout: 'default',
	requiresAuth: true
})

const appStore = useAppStore()
const usersStore = useUsersStore()

onMounted(() => {
	usersStore.fetchUsers()
})

const handleSuccessNotification = (message: string) => {
	appStore.notificationItems.unshift({
		title: 'User Management',
		description: message,
		type: 'success',
		variant: 'outline',
		dateAdded: new Date()
	})
}
</script>

<template>
	<div class="p-4 md:p-6 w-full">
		<AppLayoutPageContainer>
			<!-- Header -->
			<AppLayoutPageHeader
				title="User Management"
				description="Overview of platform access and user credentials across clusters."
			>
				<template #actions>
					<UiButton
						variant="outline"
						class="gap-1.5 shadow-xs"
						:disabled="usersStore.loading.table"
						@click="usersStore.fetchUsers()"
					>
						<UiIcon
							name="refresh"
							size="16px"
							:class="{ 'animate-spin': usersStore.loading.table }"
						/>
						<span>Refresh</span>
					</UiButton>
				</template>
			</AppLayoutPageHeader>

			<!-- Error Alert -->
			<div
				v-if="usersStore.error"
				class="p-3 rounded-xl bg-destructive/10 text-destructive border border-destructive/20 text-xs flex items-center justify-between"
			>
				<div class="flex items-center gap-2">
					<UiIcon name="alert-circle-outline" size="16px" />
					<span>{{ usersStore.error }}</span>
				</div>
				<UiButton
					variant="outline"
					size="sm"
					class="h-7 text-xs border-destructive/30 text-destructive hover:bg-destructive/10"
					@click="usersStore.fetchUsers()"
				>
					Retry
				</UiButton>
			</div>

			<!-- 1. Stats Summary Cards -->
			<div>
				<UserStatsCards />
			</div>

			<!-- 2. Filters & Actions Toolbar -->
			<div>
				<UserToolbar />
			</div>

			<!-- 3. Users Table -->
			<div class="w-full">
				<UserTable />
			</div>

			<!-- 4. Modals -->
			<ModalUserDetail />
			<ModalUserForm @success="handleSuccessNotification" />
			<ModalDeleteUser @success="handleSuccessNotification" />
		</AppLayoutPageContainer>
	</div>
</template>

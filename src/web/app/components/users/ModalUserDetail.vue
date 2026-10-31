<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed } from 'vue'
import { storeToRefs } from 'pinia'
import { useUsersStore } from '@/stores/users.store'
import { useAppStore } from '@/stores/app.store'
import appHelper from '@/app/helpers/app.helper'
import { formatUtcDate } from '@/app/helpers/date.helper'

const usersStore = useUsersStore()
const appStore = useAppStore()
const { modalDetailShow, currentUser } = storeToRefs(usersStore)

const close = () => {
	usersStore.toggleModalDetail(false)
}

const isUserActive = computed(() => Number(currentUser.value?.adm_status ?? 1) === 1)

const defaultClusterDisplay = computed(() => {
	const activeUrl = appStore.activeClusterURL || appHelper.getDefaultClusterValue()
	return activeUrl ? appHelper.getClusterName(activeUrl) || activeUrl : 'All Clusters'
})
</script>

<template>
	<UiDialog v-model:open="modalDetailShow">
		<UiDialogContent class="max-w-md p-6 max-h-[85vh] flex flex-col gap-4">
			<UiDialogHeader>
				<div class="flex items-center gap-3">
					<div
						class="flex items-center justify-center p-2 rounded-lg bg-sky-100 text-sky-600 dark:bg-sky-950/50 dark:text-sky-400"
					>
						<UiIcon name="account" size="20px" />
					</div>
					<UiDialogTitle class="text-lg font-semibold text-foreground">
						Read One User
					</UiDialogTitle>
				</div>
			</UiDialogHeader>

			<div v-if="currentUser" class="flex flex-col gap-3 py-2 text-sm flex-1 overflow-y-auto">
				<div class="flex flex-row items-center">
					<div class="text-sm font-medium text-muted-foreground w-[140px]">User ID</div>
					<div class="text-sm font-mono text-xs text-foreground truncate flex-1">
						: {{ currentUser.user_id }}
					</div>
				</div>

				<div class="flex flex-row items-center">
					<div class="text-sm font-medium text-muted-foreground w-[140px]">Username</div>
					<div class="text-sm font-semibold text-foreground flex-1">
						: @{{ currentUser.username }}
					</div>
				</div>

				<div class="flex flex-row items-center">
					<div class="text-sm font-medium text-muted-foreground w-[140px]">
						Display Name
					</div>
					<div class="text-sm font-normal text-foreground flex-1">
						: {{ currentUser.display_name || '-' }}
					</div>
				</div>

				<div class="flex flex-row items-center">
					<div class="text-sm font-medium text-muted-foreground w-[140px]">Email</div>
					<div class="text-sm font-normal text-foreground flex-1">
						: {{ currentUser.email || '-' }}
					</div>
				</div>

				<div class="flex flex-row items-center">
					<div class="text-sm font-medium text-muted-foreground w-[140px]">Cluster</div>
					<div class="text-sm font-normal text-foreground flex-1">
						:
						<template v-if="currentUser.clusters && currentUser.clusters.length > 0">
							<span
								v-for="(c, idx) in currentUser.clusters"
								:key="idx"
								class="inline-flex items-center px-2 py-0.5 rounded text-xs font-medium bg-secondary text-secondary-foreground border mr-1"
							>
								{{ appHelper.getClusterName(c) || c }}
							</span>
						</template>
						<template v-else-if="currentUser.cluster_name || currentUser.cluster_url">
							<span
								class="inline-flex items-center px-2 py-0.5 rounded text-xs font-medium bg-secondary text-secondary-foreground border"
							>
								{{
									appHelper.getClusterName(
										currentUser.cluster_name || currentUser.cluster_url
									)
								}}
							</span>
						</template>
						<span
							v-else
							class="inline-flex items-center px-2 py-0.5 rounded text-xs font-medium bg-muted text-muted-foreground border"
						>
							{{ defaultClusterDisplay }}
						</span>
					</div>
				</div>

				<div class="flex flex-row items-center">
					<div class="text-sm font-medium text-muted-foreground w-[140px]">Status</div>
					<div class="text-sm font-normal flex items-center gap-2 flex-1">
						:
						<div
							class="w-2.5 h-2.5 rounded-full"
							:class="isUserActive ? 'bg-emerald-500' : 'bg-destructive'"
						></div>
						<span>{{ isUserActive ? 'Active' : 'Inactive' }}</span>
					</div>
				</div>

				<div class="flex flex-row items-center">
					<div class="text-sm font-medium text-muted-foreground w-[140px]">Allow SSO</div>
					<div class="text-sm font-normal flex-1">
						:
						<span
							v-if="Boolean(currentUser.allow_sso)"
							class="inline-flex items-center px-2 py-0.5 rounded text-xs font-medium bg-sky-50 text-sky-700 border border-sky-200 dark:bg-sky-950/40 dark:text-sky-300 dark:border-sky-800 ml-1"
						>
							Allowed
						</span>
						<span
							v-else
							class="inline-flex items-center px-2 py-0.5 rounded text-xs font-medium bg-muted text-muted-foreground border ml-1"
						>
							Not Allowed
						</span>
					</div>
				</div>

				<div class="flex flex-row items-center">
					<div class="text-sm font-medium text-muted-foreground w-[140px]">
						Last Updated
					</div>
					<div class="text-sm font-normal text-muted-foreground flex-1">
						:
						{{
							currentUser.last_updated ? formatUtcDate(currentUser.last_updated) : '-'
						}}
					</div>
				</div>
			</div>

			<UiDialogFooter class="flex items-center justify-end pt-3 border-t">
				<UiButton variant="outline" size="sm" @click="close"> Close </UiButton>
			</UiDialogFooter>
		</UiDialogContent>
	</UiDialog>
</template>

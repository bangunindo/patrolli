<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed } from 'vue'
import AppTable from '@/components/app/AppTable.vue'
import { useUsersStore } from '@/stores/users.store'
import { useAppStore } from '@/stores/app.store'
import appHelper from '@/app/helpers/app.helper'
import type { IUserRecord } from '@@/shared/types/user.type'
import { storeToRefs } from 'pinia'

const usersStore = useUsersStore()
const appStore = useAppStore()
const { filteredUsers, loading, pagination } = storeToRefs(usersStore)

const columns = [
	{
		key: 'user',
		title: 'USERNAME',
		sortable: true,
		width: '240px'
	},
	{
		key: 'email',
		title: 'EMAIL',
		width: '200px'
	},
	{
		key: 'cluster',
		title: 'CLUSTER',
		width: '180px'
	},
	{
		key: 'allow_sso',
		title: 'ALLOW SSO',
		width: '130px'
	},
	{
		key: 'last_updated',
		title: 'LAST UPDATED',
		sortable: true,
		width: '170px'
	},
	{
		key: 'status',
		title: 'STATUS',
		width: '110px'
	}
]

const getClusterDisplay = (val?: string): string => {
	if (!val) return ''
	return appHelper.getClusterName(val) || val
}

const defaultClusterName = computed(() => {
	const activeUrl = appStore.activeClusterURL || appHelper.getDefaultClusterValue()
	return activeUrl ? appHelper.getClusterName(activeUrl) || activeUrl : 'All Clusters'
})

const getInitials = (name?: string | null): string => {
	if (!name) return 'U'
	const parts = name.trim().split(/\s+/)
	if (parts.length >= 2) {
		return (parts[0][0] + parts[1][0]).toUpperCase()
	}
	return name.substring(0, 2).toUpperCase()
}

const getAvatarBg = (username: string): string => {
	const colors = [
		'bg-blue-100 text-blue-700 dark:bg-blue-900/50 dark:text-blue-300',
		'bg-purple-100 text-purple-700 dark:bg-purple-900/50 dark:text-purple-300',
		'bg-emerald-100 text-emerald-700 dark:bg-emerald-900/50 dark:text-emerald-300',
		'bg-amber-100 text-amber-700 dark:bg-amber-900/50 dark:text-amber-300',
		'bg-rose-100 text-rose-700 dark:bg-rose-900/50 dark:text-rose-300'
	]
	let hash = 0
	for (let i = 0; i < username.length; i++) {
		hash = username.charCodeAt(i) + ((hash << 5) - hash)
	}
	const index = Math.abs(hash) % colors.length
	return colors[index]
}

const formatDate = (isoString?: string | null): string => {
	if (!isoString) return '-'
	try {
		return new Date(isoString).toLocaleString()
	} catch {
		return isoString
	}
}

const onPagination = (payload: { limit: number; offset: number; page: number }) => {
	usersStore.filter.limit = payload.limit
	usersStore.filter.offset = payload.offset
	usersStore.pagination.currentPage = payload.page
	usersStore.pagination.rows = payload.limit
	usersStore.fetchUsers()
}

const onSort = (payload: { key: string | null; order: 'asc' | 'desc' | null }) => {
	if (payload.key === 'user') {
		usersStore.filter.order_by = 'username'
	} else if (payload.key === 'last_updated') {
		usersStore.filter.order_by = 'last_updated'
	} else {
		usersStore.filter.order_by = undefined
	}
	usersStore.filter.order_dir = payload.order ?? undefined
	usersStore.fetchUsers()
}

const onView = (user: IUserRecord) => {
	usersStore.toggleModalDetail(true, user)
}

const onEdit = (user: IUserRecord) => {
	usersStore.toggleModalForm(true, user)
}

const onDeactivate = (user: IUserRecord) => {
	const action = Number(user.adm_status) === 1 ? 'deactivate' : 'activate'
	usersStore.toggleModalDelete(true, user, action)
}

const onDelete = (user: IUserRecord) => {
	usersStore.toggleModalDelete(true, user, 'delete')
}
</script>

<template>
	<div>
		<AppTable
			:columns="columns"
			:data="filteredUsers"
			:loading="loading.table"
			:use-selection="false"
			:use-action="true"
			:pagination="pagination"
			entity-label="users"
			@update:page="onPagination"
			@update:sort="onSort"
		>
			<!-- USERNAME column -->
			<template #col.user="{ data }">
				<div class="flex items-center gap-3 py-1">
					<div
						class="w-9 h-9 rounded-full flex items-center justify-center font-bold text-xs shrink-0"
						:class="getAvatarBg(data.username)"
					>
						{{ getInitials(data.display_name || data.username) }}
					</div>
					<div class="flex flex-col min-w-0">
						<span
							class="text-sm font-semibold text-gray-900 dark:text-gray-100 truncate"
						>
							{{ data.display_name || data.username }}
						</span>
						<span class="text-xs text-gray-500 truncate"> @{{ data.username }} </span>
					</div>
				</div>
			</template>

			<!-- EMAIL column -->
			<template #col.email="{ data }">
				<span class="text-xs text-gray-600 dark:text-gray-400 truncate">
					{{ data.email || '-' }}
				</span>
			</template>

			<!-- CLUSTER column -->
			<template #col.cluster="{ data }">
				<div class="flex flex-wrap items-center gap-1.5 py-1">
					<span
						v-if="data.cluster_name || data.cluster_url"
						class="inline-flex items-center px-2 py-0.5 rounded text-xs font-medium bg-slate-100 text-slate-700 dark:bg-slate-800 dark:text-slate-300 border border-slate-200 dark:border-slate-700"
					>
						{{ getClusterDisplay(data.cluster_name || data.cluster_url) }}
					</span>
					<template v-else-if="data.clusters && data.clusters.length > 0">
						<span
							v-for="(c, idx) in data.clusters"
							:key="idx"
							class="inline-flex items-center px-2 py-0.5 rounded text-xs font-medium bg-slate-100 text-slate-700 dark:bg-slate-800 dark:text-slate-300 border border-slate-200 dark:border-slate-700"
						>
							{{ getClusterDisplay(c) }}
						</span>
					</template>
					<span
						v-else
						class="inline-flex items-center px-2 py-0.5 rounded text-xs font-medium bg-gray-50 text-gray-600 dark:bg-gray-800/60 dark:text-gray-400 border border-gray-200 dark:border-gray-700"
					>
						{{ defaultClusterName }}
					</span>
				</div>
			</template>

			<!-- ALLOW SSO column -->
			<template #col.allow_sso="{ data }">
				<span
					v-if="Boolean(data.allow_sso)"
					class="inline-flex items-center px-2 py-0.5 rounded text-xs font-medium bg-sky-50 text-sky-700 border border-sky-200 dark:bg-sky-950/40 dark:text-sky-300 dark:border-sky-800"
				>
					Allowed
				</span>
				<span
					v-else
					class="inline-flex items-center px-2 py-0.5 rounded text-xs font-medium bg-gray-100 text-gray-700 border border-gray-200 dark:bg-gray-800 dark:text-gray-300 dark:border-gray-700"
				>
					Not Allowed
				</span>
			</template>

			<!-- LAST UPDATED column -->
			<template #col.last_updated="{ data }">
				<span class="text-xs text-gray-600 dark:text-gray-400">
					{{ formatDate(data.last_updated) }}
				</span>
			</template>

			<!-- STATUS column -->
			<template #col.status="{ data }">
				<div class="flex items-center gap-1.5">
					<span
						class="w-2 h-2 rounded-full"
						:class="data.adm_status === 1 ? 'bg-emerald-500' : 'bg-gray-400'"
					></span>
					<span
						class="text-xs font-medium"
						:class="
							data.adm_status === 1
								? 'text-emerald-700 dark:text-emerald-400'
								: 'text-gray-500'
						"
					>
						{{ data.adm_status === 1 ? 'Active' : 'Inactive' }}
					</span>
				</div>
			</template>

			<!-- ACTIONS column matching Figma node 1764:35330 -->
			<template #col.action="prop">
				<div class="flex flex-row justify-end">
					<UiDropdownMenu>
						<UiDropdownMenuTrigger as-child>
							<UiButton size="icon" variant="outline" class="h-8 w-8">
								<UiIcon name="dots-vertical" class="w-4 h-4" />
							</UiButton>
						</UiDropdownMenuTrigger>

						<UiDropdownMenuContent align="end" class="w-48 p-1.5">
							<UiDropdownMenuItem
								class="cursor-pointer text-sm"
								@click="onView(prop.data)"
							>
								<span>View</span>
							</UiDropdownMenuItem>
							<UiDropdownMenuItem
								class="cursor-pointer text-sm"
								@click="onEdit(prop.data)"
							>
								<span>Edit</span>
							</UiDropdownMenuItem>
							<UiDropdownMenuItem
								class="cursor-pointer text-sm"
								@click="onDeactivate(prop.data)"
							>
								<span>{{
									Number(prop.data.adm_status) === 1
										? 'Deactivate Account'
										: 'Activate Account'
								}}</span>
							</UiDropdownMenuItem>
							<UiDropdownMenuItem
								class="cursor-pointer text-sm text-destructive focus:text-destructive font-medium"
								@click="onDelete(prop.data)"
							>
								<span>Delete</span>
							</UiDropdownMenuItem>
						</UiDropdownMenuContent>
					</UiDropdownMenu>
				</div>
			</template>
		</AppTable>
	</div>
</template>

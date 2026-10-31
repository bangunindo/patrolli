<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { useUsersStore } from '@/stores/users.store'
import { storeToRefs } from 'pinia'

const usersStore = useUsersStore()
const { stats } = storeToRefs(usersStore)
</script>

<template>
	<div class="grid grid-cols-1 md:grid-cols-3 gap-4 mb-4">
		<!-- Card 1: Total Allow SSO Users -->
		<div
			class="bg-white dark:bg-[#1e1e1e] border border-gray-200 dark:border-gray-700 rounded-xl p-4 shadow-sm flex flex-col justify-between"
		>
			<div class="text-xs font-semibold text-gray-500 uppercase tracking-wider mb-2">
				Total Allow SSO Users
			</div>
			<div class="flex items-baseline gap-2">
				<span class="text-2xl font-bold text-gray-900 dark:text-gray-100">
					{{ stats.totalAllowSso }}
				</span>
				<span class="text-sm font-medium text-gray-500">Users</span>
			</div>
		</div>

		<!-- Card 2: Total Disabled Users -->
		<div
			class="bg-white dark:bg-[#1e1e1e] border border-gray-200 dark:border-gray-700 rounded-xl p-4 shadow-sm flex flex-col justify-between"
		>
			<div class="text-xs font-semibold text-gray-500 uppercase tracking-wider mb-2">
				Total Disabled Users
			</div>
			<div class="flex items-baseline gap-2">
				<span class="text-2xl font-bold text-gray-900 dark:text-gray-100">
					{{ stats.totalDisabled }}
				</span>
				<span class="text-sm font-medium text-gray-500">Users</span>
			</div>
		</div>

		<!-- Card 3: Users per Cluster/Module -->
		<div
			class="bg-white dark:bg-[#1e1e1e] border border-gray-200 dark:border-gray-700 rounded-xl p-4 shadow-sm flex flex-col justify-between"
		>
			<div class="text-xs font-semibold text-gray-500 uppercase tracking-wider mb-2">
				Users per Cluster/Module
			</div>
			<div class="flex flex-wrap gap-2 items-center">
				<span
					v-for="(count, clusterName) in stats.usersPerCluster"
					:key="clusterName"
					class="inline-flex items-center px-2.5 py-1 rounded-md text-xs font-medium bg-sky-50 text-sky-700 border border-sky-200 dark:bg-sky-950/40 dark:text-sky-300 dark:border-sky-800"
				>
					{{ clusterName }}: {{ count }} Users
				</span>
				<span
					v-if="!Object.keys(stats.usersPerCluster || {}).length"
					class="text-xs text-gray-400 italic"
				>
					No cluster data
				</span>
			</div>
		</div>
	</div>
</template>

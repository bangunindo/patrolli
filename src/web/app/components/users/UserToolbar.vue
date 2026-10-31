<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import AppInputText from '@/components/app/input/AppInputText.vue'
import { useUsersStore } from '@/stores/users.store'
import { watchDebounced } from '@vueuse/core'

const usersStore = useUsersStore()

const onAddUser = () => {
	usersStore.toggleModalForm(true)
}

watchDebounced(
	() => usersStore.filter.username,
	() => {
		usersStore.filter.offset = 0
		usersStore.pagination.currentPage = 1
		usersStore.fetchUsers()
	},
	{
		debounce: 500
	}
)
</script>

<template>
	<div class="flex flex-col sm:flex-row gap-3 items-center justify-between w-full">
		<!-- Left: Search User matching Host table search -->
		<div class="w-full sm:w-80">
			<AppInputText
				v-model="usersStore.filter.username"
				variant="outline"
				placeholder="Search by username ..."
				post-icon="magnify"
			/>
		</div>

		<!-- Right: Action Button matching Host / Endpoint / Pipeline toolbar -->
		<div class="flex items-center gap-2 self-end sm:self-auto">
			<UiButton variant="outline" class="text-primary" @click="onAddUser">
				<UiIcon name="plus" class="w-4 h-4 mr-2" />
				Create User
			</UiButton>
		</div>
	</div>
</template>

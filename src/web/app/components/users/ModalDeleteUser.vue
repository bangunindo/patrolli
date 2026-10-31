<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import appHelper from '@/app/helpers/app.helper'
import { useAppStore } from '@/stores/app.store'
import { useUsersStore } from '@/stores/users.store'
import type { IUserRecord } from '@@/shared/types/user.type'
import { storeToRefs } from 'pinia'
import { computed, ref, watch } from 'vue'

const emit = defineEmits<{
	success: [message: string]
}>()

const usersStore = useUsersStore()
const appStore = useAppStore()
const { modalDeleteShow, currentUser, loading, modalDeleteActionType } = storeToRefs(usersStore)
const { activeClusterURL } = storeToRefs(appStore)

type ActionType = 'deactivate' | 'activate' | 'delete'
const actionType = ref<ActionType>('deactivate')

// Reset selected action when modal opens based on store preset or current user's active status
watch(
	[modalDeleteShow, currentUser, modalDeleteActionType],
	([isOpen, user, presetAction]) => {
		if (isOpen && user) {
			const u = user as IUserRecord
			if (presetAction) {
				actionType.value = presetAction
			} else {
				actionType.value = u.adm_status === 1 ? 'deactivate' : 'delete'
			}
		}
	},
	{ immediate: true }
)

const isUserActive = computed(() => (currentUser.value?.adm_status ?? 1) === 1)

const assignedClustersList = computed(() => {
	const c =
		currentUser.value?.cluster_name ||
		currentUser.value?.cluster_url ||
		(currentUser.value?.clusters && currentUser.value.clusters[0])
	if (c) {
		const displayName = appHelper.getClusterName(c) || c
		return `(${displayName})`
	}
	return ''
})

const actionButtonText = computed(() => {
	if (actionType.value === 'deactivate') return 'Deactivate User'
	if (actionType.value === 'activate') return 'Activate User'
	return 'Delete User'
})

const errorMessage = ref<string | null>(null)

watch(modalDeleteShow, (val) => {
	if (val) errorMessage.value = null
})

const actionButtonColor = computed(() => {
	if (actionType.value === 'activate') return 'primary'
	return 'error'
})

const close = () => {
	usersStore.toggleModalDelete(false)
}

const confirmAction = async () => {
	if (!currentUser.value) return
	errorMessage.value = null
	const name = currentUser.value.display_name || currentUser.value.username
	const clusterVal =
		currentUser.value.cluster_url ||
		currentUser.value.cluster_name ||
		(currentUser.value.clusters && currentUser.value.clusters[0]) ||
		activeClusterURL.value ||
		appHelper.getDefaultClusterValue()
	const targetClusters = clusterVal ? [clusterVal] : undefined

	try {
		if (actionType.value === 'deactivate') {
			await usersStore.toggleUserStatus(
				currentUser.value.user_id,
				0,
				targetClusters,
				currentUser.value.username
			)
			usersStore.toggleModalDelete(false)
			emit('success', `User “${name}” has been deactivated.`)
		} else if (actionType.value === 'activate') {
			await usersStore.toggleUserStatus(
				currentUser.value.user_id,
				1,
				targetClusters,
				currentUser.value.username
			)
			usersStore.toggleModalDelete(false)
			emit('success', `User “${name}” has been activated.`)
		} else if (actionType.value === 'delete') {
			await usersStore.deleteUser(
				currentUser.value.user_id,
				targetClusters,
				currentUser.value.username
			)
			usersStore.toggleModalDelete(false)
			emit('success', `User “${name}” has been deleted.`)
		}
	} catch (err: any) {
		errorMessage.value = err.message || 'Operation failed'
	}
}
</script>

<template>
	<UiDialog v-model:open="modalDeleteShow">
		<UiDialogContent class="max-w-md p-6 flex flex-col gap-4">
			<UiDialogHeader>
				<div class="flex items-center gap-3">
					<div
						class="w-10 h-10 rounded-full flex items-center justify-center bg-destructive/10 text-destructive shrink-0"
					>
						<UiIcon name="alert-circle-outline" size="20px" />
					</div>
					<UiDialogTitle class="text-lg font-semibold text-foreground">
						Confirm Action
					</UiDialogTitle>
				</div>
			</UiDialogHeader>

			<!-- Error Alert if any -->
			<div
				v-if="errorMessage"
				class="p-3 rounded-lg bg-destructive/10 text-destructive border border-destructive/20 text-xs"
			>
				{{ errorMessage }}
			</div>

			<!-- Context Description -->
			<p class="text-sm text-muted-foreground leading-relaxed -mt-1">
				<template v-if="modalDeleteActionType !== 'delete'">
					You are about to {{ isUserActive ? 'deactivate' : 'activate' }} user
					<strong class="text-foreground font-bold">
						{{ currentUser?.username || currentUser?.display_name }}</strong
					>. Please confirm below:
				</template>
				<template v-else>
					You are about to take action on user
					<strong class="text-foreground font-bold">
						{{ currentUser?.username || currentUser?.display_name }}</strong
					>. Please select an option:
				</template>
			</p>

			<!-- Radio Option Cards -->
			<div class="flex flex-col gap-2.5">
				<!-- Option 1: Deactivate (or Activate) -->
				<div
					@click="actionType = isUserActive ? 'deactivate' : 'activate'"
					class="p-3 rounded-lg border transition-all cursor-pointer flex items-start gap-3"
					:class="[
						(isUserActive && actionType === 'deactivate') ||
						(!isUserActive && actionType === 'activate')
							? 'bg-sky-50/80 dark:bg-sky-950/40 border-sky-500 shadow-sm'
							: 'bg-card border-border hover:border-muted-foreground/40'
					]"
				>
					<!-- Radio Indicator -->
					<div class="mt-0.5 shrink-0 flex items-center justify-center">
						<div
							class="w-4 h-4 rounded-full border flex items-center justify-center transition-colors"
							:class="[
								(isUserActive && actionType === 'deactivate') ||
								(!isUserActive && actionType === 'activate')
									? 'border-sky-600 bg-sky-600'
									: 'border-muted-foreground/40 bg-card'
							]"
						>
							<div
								v-if="
									(isUserActive && actionType === 'deactivate') ||
									(!isUserActive && actionType === 'activate')
								"
								class="w-1.5 h-1.5 rounded-full bg-white"
							></div>
						</div>
					</div>

					<!-- Option Text -->
					<div class="flex flex-col gap-0.5">
						<span class="text-sm font-semibold text-foreground">
							{{ isUserActive ? 'Deactivate Account' : 'Activate Account' }}
						</span>
						<span class="text-xs text-muted-foreground leading-relaxed">
							{{
								isUserActive
									? 'Temporarily disable this account. User data and settings will be preserved.'
									: 'Re-enable this account. User data and access across clusters will be restored.'
							}}
						</span>
					</div>
				</div>

				<!-- Option 2: Delete Permanently (Only shown for delete action) -->
				<div
					v-if="modalDeleteActionType === 'delete'"
					@click="actionType = 'delete'"
					class="p-3 rounded-lg border transition-all cursor-pointer flex items-start gap-3"
					:class="[
						actionType === 'delete'
							? 'bg-destructive/10 border-destructive shadow-sm'
							: 'bg-card border-border hover:border-muted-foreground/40'
					]"
				>
					<!-- Radio Indicator -->
					<div class="mt-0.5 shrink-0 flex items-center justify-center">
						<div
							class="w-4 h-4 rounded-full border flex items-center justify-center transition-colors"
							:class="[
								actionType === 'delete'
									? 'border-destructive bg-destructive'
									: 'border-muted-foreground/40 bg-card'
							]"
						>
							<div
								v-if="actionType === 'delete'"
								class="w-1.5 h-1.5 rounded-full bg-white"
							></div>
						</div>
					</div>

					<!-- Option Text -->
					<div class="flex flex-col gap-0.5">
						<span class="text-sm font-semibold text-destructive">
							Delete Permanently
						</span>
						<span class="text-xs text-muted-foreground leading-relaxed">
							Remove this user and all associated data. This action cannot be undone.
						</span>
					</div>
				</div>
			</div>

			<!-- Warning Banner -->
			<div
				class="bg-amber-500/10 border border-amber-500/30 rounded-lg p-3 flex items-start gap-2.5"
			>
				<UiIcon
					name="alert"
					size="16px"
					class="text-amber-600 dark:text-amber-400 shrink-0 mt-0.5"
				/>
				<p
					class="text-xs font-medium text-amber-900 dark:text-amber-200 leading-relaxed mb-0"
				>
					This action will affect the user across all assigned clusters
					<span class="font-bold">{{ assignedClustersList }}</span
					>.
				</p>
			</div>

			<!-- Modal Footer -->
			<UiDialogFooter class="flex items-center justify-end gap-2 pt-3 border-t">
				<UiButton variant="outline" size="sm" :disabled="loading.form" @click="close">
					Cancel
				</UiButton>
				<UiButton
					:variant="actionButtonColor === 'error' ? 'destructive' : 'default'"
					size="sm"
					:disabled="loading.form"
					@click="confirmAction"
				>
					<UiIcon
						v-if="loading.form"
						name="refresh"
						size="14px"
						class="animate-spin mr-1.5"
					/>
					<span>{{ actionButtonText }}</span>
				</UiButton>
			</UiDialogFooter>
		</UiDialogContent>
	</UiDialog>
</template>

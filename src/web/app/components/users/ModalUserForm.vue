<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import appHelper from '@/app/helpers/app.helper'
import cookieHelper from '@/app/helpers/cookie.helper'
import tokenHelper from '@/app/helpers/token.helper'
import { userHints } from '@/app/services/forms/ui-hints/user.hints'
import DynamicZodForm from '@/components/app/dynamic-zod-form/DynamicZodForm.vue'
import AppInputCluster from '@/components/app/input/AppInputCluster.vue'
import { useAppStore } from '@/stores/app.store'
import { useAuthStore } from '@/stores/auth.store'
import { useUsersStore } from '@/stores/users.store'
import { FormMode } from '@@/shared/types/app.type'
import { CreateUserFormSchema, UpdateUserFormSchema } from '@@/shared/types/user.type'
import { storeToRefs } from 'pinia'
import { ref, watch } from 'vue'

const emit = defineEmits<{
	success: [message: string]
}>()

const usersStore = useUsersStore()
const authStore = useAuthStore()
const appStore = useAppStore()
const { modalFormShow, userFormMode, currentUser, loading } = storeToRefs(usersStore)
const { isClusterReadonly } = storeToRefs(authStore)
const { activeClusterURL } = storeToRefs(appStore)

const dynamicFormRef = ref<InstanceType<typeof DynamicZodForm> | null>(null)
const errorMessage = ref<string | null>(null)

const form = ref<Record<string, any>>({
	display_name: '',
	username: '',
	email: '',
	password: '',
	allow_sso: true,
	adm_status: 1
})

const selectedCluster = ref<any>([])

// Reset / Initialize Form when modal opens or currentUser changes
watch(
	modalFormShow,
	(isOpen) => {
		if (isOpen) {
			errorMessage.value = null
			if (userFormMode.value === FormMode.UPDATE && currentUser.value) {
				const currentCluster =
					currentUser.value.cluster_url ||
					currentUser.value.cluster_name ||
					(currentUser.value.clusters && currentUser.value.clusters[0]) ||
					appHelper.getDefaultClusterValue()

				selectedCluster.value = currentCluster

				form.value = {
					display_name: currentUser.value.display_name ?? '',
					username: currentUser.value.username ?? '',
					email: currentUser.value.email ?? '',
					password: '',
					allow_sso:
						currentUser.value.allow_sso !== undefined
							? Boolean(currentUser.value.allow_sso)
							: true,
					adm_status: currentUser.value.adm_status ?? 1
				}
			} else {
				const activeUrl = activeClusterURL.value || appHelper.getDefaultClusterValue()
				selectedCluster.value = activeUrl ? [activeUrl] : []

				form.value = {
					display_name: '',
					username: '',
					email: '',
					password: '',
					allow_sso: true,
					adm_status: 1
				}
			}
		}
	},
	{ immediate: true }
)

const handleFormSubmit = async (validFormData: any) => {
	errorMessage.value = null
	const username = validFormData.username || form.value.username
	const displayName = validFormData.display_name || username
	const payload = { ...validFormData, username }

	if (userFormMode.value === FormMode.UPDATE && !payload.password) {
		delete payload.password
	}

	try {
		if (userFormMode.value === FormMode.ADD) {
			const clusterURLs = Array.isArray(selectedCluster.value)
				? selectedCluster.value
				: selectedCluster.value
					? [selectedCluster.value]
					: []

			if (clusterURLs.length === 0) {
				errorMessage.value = 'Please select at least one target cluster.'
				return
			}

			await usersStore.createUser(payload, undefined, clusterURLs)
			usersStore.toggleModalForm(false)
			emit('success', `User “${displayName}” has been created.`)
		} else if (currentUser.value) {
			const userId = currentUser.value.user_id
			const targetCluster =
				currentUser.value.cluster_url ||
				currentUser.value.cluster_name ||
				(typeof selectedCluster.value === 'string'
					? selectedCluster.value
					: selectedCluster.value?.[0])
			const clusterURLs = targetCluster ? [targetCluster] : undefined

			const prevAllowSso = Boolean(currentUser.value.allow_sso)
			const prevSsoLinked = Boolean(currentUser.value.sso_linked)
			const newAllowSso = Boolean(payload.allow_sso)
			const allowSsoChanged = prevAllowSso !== newAllowSso

			// If allow_sso was previously true and is updated to false, call API to unlink SSO
			if ((prevAllowSso || prevSsoLinked) && !payload.allow_sso) {
				await usersStore.unlinkSso(userId, clusterURLs, currentUser.value.username)
			}
			await usersStore.updateUser(userId, payload, undefined, clusterURLs)

			// If allow_sso changes for the currently authenticated user, refresh user clusters & global filter
			if (allowSsoChanged) {
				const decoded = tokenHelper.decodeToken() as any
				const idToken = cookieHelper.get('id_token')
				const decodedId = idToken ? (tokenHelper.decodeToken(idToken) as any) : null

				const currentIdentifiers = [
					decoded?.preferred_username,
					decoded?.username,
					decoded?.sub,
					decoded?.email,
					decodedId?.preferred_username,
					decodedId?.username,
					decodedId?.sub,
					decodedId?.email
				]
					.filter(Boolean)
					.map((v) => String(v).toLowerCase())

				const targetIdentifiers = [
					currentUser.value.username,
					currentUser.value.email,
					currentUser.value.user_id
				]
					.filter(Boolean)
					.map((v) => String(v).toLowerCase())

				const isCurrentLoggedInUser =
					currentIdentifiers.some((id) => targetIdentifiers.includes(id)) ||
					(currentIdentifiers.length === 0 &&
						currentUser.value.username?.toLowerCase() === 'admin')

				if (isCurrentLoggedInUser) {
					await authStore.refreshUserClusters()
				}
			}

			usersStore.toggleModalForm(false)
			emit('success', `User “${displayName}” updated successfully.`)
		}
	} catch (err: any) {
		errorMessage.value = err.message || 'Operation failed'
	}
}

const triggerSubmit = () => {
	dynamicFormRef.value?.validateAndSubmit()
}

const closeModal = () => {
	usersStore.toggleModalForm(false)
}
</script>

<template>
	<UiDialog v-model:open="modalFormShow">
		<UiDialogContent class="max-w-lg p-6 max-h-[85vh] flex flex-col gap-4">
			<!-- Header -->
			<UiDialogHeader>
				<div class="flex items-center gap-3">
					<div
						class="flex items-center justify-center p-2 rounded-lg bg-sky-100 text-sky-600 dark:bg-sky-950/50 dark:text-sky-400"
					>
						<UiIcon name="account" size="20px" />
					</div>
					<UiDialogTitle class="text-lg font-semibold text-foreground">
						{{ userFormMode === FormMode.ADD ? 'Create User' : 'Edit User' }}
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

			<!-- Dynamic Zod Form with Tight Section Groupings -->
			<div class="flex flex-col gap-3 flex-1 overflow-y-auto">
				<DynamicZodForm
					ref="dynamicFormRef"
					v-model="form"
					:schema="
						userFormMode === FormMode.ADD ? CreateUserFormSchema : UpdateUserFormSchema
					"
					:hints="userHints"
					:disabled="loading.form"
					:loading="loading.form"
					@submit="handleFormSubmit"
				/>

				<!-- Cluster Selector matching host form -->
				<AppInputCluster
					v-model="selectedCluster"
					label="Cluster"
					required
					:multiple="userFormMode === FormMode.ADD"
					:disabled="userFormMode === FormMode.UPDATE || isClusterReadonly"
				/>
			</div>

			<!-- Footer Buttons -->
			<UiDialogFooter class="flex items-center justify-end gap-2 pt-3 border-t">
				<UiButton
					type="button"
					variant="outline"
					:disabled="loading.form"
					@click="closeModal"
				>
					Cancel
				</UiButton>
				<UiButton type="button" :disabled="loading.form" @click="triggerSubmit">
					<UiIcon
						v-if="loading.form"
						name="refresh"
						size="16px"
						class="animate-spin mr-1.5"
					/>
					<span>Save</span>
				</UiButton>
			</UiDialogFooter>
		</UiDialogContent>
	</UiDialog>
</template>

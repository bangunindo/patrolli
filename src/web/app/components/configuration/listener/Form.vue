<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { onMounted, ref, watch } from 'vue'
import { storeToRefs } from 'pinia'
import appHelper from '@/app/helpers/app.helper'
import { useAuthStore } from '@/stores/auth.store'
import { useListenerStore } from '@/stores/listeners.store'
import { FormMode } from '@@/shared/types/app.type'
import { CreateListenerRequest } from '@@/shared/generated/patrolliAPI.schemas.type'
import { listenerHints } from '@/app/services/forms/ui-hints/listener.hints'
import type { IListenerForm } from '@@/shared/types/listener.type'

import AppInputCluster from '@/components/app/input/AppInputCluster.vue'
import AppInputText from '@/components/app/input/AppInputText.vue'
import AppInputLabel from '@/components/app/input/Label.vue'
import DynamicZodForm from '@/components/app/dynamic-zod-form/DynamicZodForm.vue'

const emits = defineEmits(['after:submit'])
const props = withDefaults(
	defineProps<{
		showTitle?: boolean
		showCancel?: boolean
		footerFull?: boolean
	}>(),
	{
		showTitle: true,
		showCancel: true,
		footerFull: false
	}
)

const listenerStore = useListenerStore()
const authStore = useAuthStore()
const { loading, listenerFormMode, currentListener } = storeToRefs(listenerStore)
const { isClusterReadonly } = storeToRefs(authStore)

const dynamicFormRef = ref<InstanceType<typeof DynamicZodForm> | null>(null)
const selectedCluster = ref(appHelper.getDefaultClusterValue())

const form = ref<Record<string, any>>({
	name: '',
	adm_status: 1,
	labels: JSON.stringify({}),
	config: {
		type: 'dummy',
		config: {
			messages_per_batch: 100,
			speed: 5
		}
	} as any,
	worker_count: 1,
	disable_cluster_forwarding: true
})

const transformFormData = (raw: Record<string, any>) => {
	const clone = JSON.parse(JSON.stringify(raw))
	if (clone.config && typeof clone.config === 'object') {
		if (clone.config.type === 'dummy') {
			const dummyCfg = clone.config.config || {}
			clone.config.config = {
				messages_per_batch:
					dummyCfg.messages_per_batch !== undefined && dummyCfg.messages_per_batch !== ''
						? Number(dummyCfg.messages_per_batch)
						: 100,
				speed:
					dummyCfg.speed !== undefined && dummyCfg.speed !== ''
						? Number(dummyCfg.speed)
						: 5
			}
		}
	}
	return clone
}

const saveOrUpdate = async (validFormData: IListenerForm) => {
	let res
	const cfg = (validFormData.config as any)?.config
	if (cfg && cfg.batch_timeout) {
		cfg.batch_timeout = parseInt(cfg.batch_timeout)
	}

	if (typeof validFormData.labels === 'string') {
		try {
			validFormData.labels = JSON.parse(validFormData.labels || '{}')
		} catch {
			validFormData.labels = {}
		}
	} else if (!validFormData.labels || typeof validFormData.labels !== 'object') {
		validFormData.labels = {}
	}

	const clusterURLs = [selectedCluster.value]

	if (listenerFormMode.value === FormMode.ADD) {
		res = await listenerStore.createListener(validFormData, clusterURLs)
	} else {
		res = await listenerStore.updateListener(
			currentListener.value?.listener_id ?? '',
			validFormData,
			clusterURLs
		)
	}
	return res
}

const handleFormSubmit = async (validFormData: any) => {
	loading.value.form = true
	try {
		const res = await saveOrUpdate(validFormData as IListenerForm)
		if (res) {
			closeFormModal()
			emits('after:submit')
		}
	} catch {
		// Handled in store
	} finally {
		loading.value.form = false
	}
}

const triggerSubmit = () => {
	dynamicFormRef.value?.validateAndSubmit()
}

const closeFormModal = () => {
	loading.value.form = false
	listenerStore.toggleListenerModalForm(false)
}

const setDefaultFormEdit = () => {
	if (!currentListener.value) return
	const rawConfig = currentListener.value.config
	form.value.name = currentListener.value.name ?? ''
	form.value.adm_status = Number(currentListener.value.adm_status ?? 1) as 0 | 1
	form.value.labels =
		typeof currentListener.value.labels === 'object' && currentListener.value.labels !== null
			? JSON.stringify(currentListener.value.labels)
			: (currentListener.value.labels as unknown as string) || JSON.stringify({})
	form.value.config = rawConfig ? JSON.parse(JSON.stringify(rawConfig)) : undefined
	form.value.worker_count = Number(currentListener.value.worker_count || 1)
	form.value.disable_cluster_forwarding = currentListener.value.disable_cluster_forwarding ?? true
}

onMounted(() => {
	const activeClusterURL = appHelper.getDefaultClusterValue()
	selectedCluster.value = appHelper.getClusterValue(
		currentListener.value?.cluster_name ||
			currentListener.value?.cluster_url ||
			activeClusterURL
	)

	if (listenerFormMode.value === FormMode.UPDATE) {
		setDefaultFormEdit()
	}
})

watch(
	() => currentListener.value,
	() => {
		if (listenerFormMode.value === FormMode.UPDATE && currentListener.value) {
			setDefaultFormEdit()
		}
	},
	{ deep: true }
)
</script>

<template>
	<div id="listener-form" class="space-y-4">
		<div v-if="showTitle" class="flex flex-row gap-3 items-center pb-2">
			<div
				class="flex flex-row justify-center items-center p-2 rounded-lg bg-sky-100 dark:bg-sky-950 text-sky-600 dark:text-sky-400"
			>
				<UiIcon name="access-point" size="20px" />
			</div>
			<h4 class="text-base font-semibold text-foreground">
				{{ listenerFormMode === FormMode.ADD ? 'Create Listener' : 'Edit Listener' }}
			</h4>
		</div>

		<div class="flex flex-col gap-2 max-h-[calc(100vh-325px)] overflow-auto p-1">
			<DynamicZodForm
				ref="dynamicFormRef"
				v-model="form"
				:initial-data="listenerFormMode === FormMode.UPDATE ? currentListener : null"
				:schema="CreateListenerRequest"
				:hints="listenerHints"
				:transform="transformFormData"
				:disabled="loading.form"
				:loading="loading.form"
				@submit="handleFormSubmit"
			>
				<template #field:name="{ error, updateModelValue }">
					<div class="flex flex-col gap-2">
						<AppInputText
							type="text"
							v-model="form.name"
							@update:model-value="(val: any) => updateModelValue?.(val)"
							variant="outline"
							placeholder="Name of listener"
							label-class="font-semibold"
							label="Name"
							required
							:error="error"
						/>
						<AppInputCluster
							v-model="selectedCluster"
							label="Cluster"
							required
							:disabled="listenerFormMode === FormMode.UPDATE || isClusterReadonly"
						/>
					</div>
				</template>

				<template #field:labels="{ error, updateModelValue }">
					<div>
						<AppInputLabel
							v-model="form.labels"
							@update:model-value="(val: any) => updateModelValue?.(val)"
							label="Labels"
							:error="error"
						/>
					</div>
				</template>
			</DynamicZodForm>
		</div>

		<div class="flex items-center justify-end gap-2 pt-4">
			<slot name="footer"></slot>
			<UiButton
				v-if="props.showCancel"
				variant="outline"
				size="sm"
				:class="{ 'flex-1 w-full': props.footerFull }"
				:disabled="loading.form"
				type="button"
				@click="closeFormModal"
			>
				Cancel
			</UiButton>
			<UiButton
				size="sm"
				:class="{ 'flex-1 w-full': props.footerFull }"
				:disabled="loading.form"
				type="button"
				@click="triggerSubmit"
			>
				<UiIcon
					v-if="loading.form"
					name="loading"
					class="animate-spin mr-1.5"
					size="14px"
				/>
				Save
			</UiButton>
		</div>
	</div>
</template>

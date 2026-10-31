<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { onMounted, ref, watch } from 'vue'
import { storeToRefs } from 'pinia'
import appHelper from '@/app/helpers/app.helper'
import AppInputCluster from '@/components/app/input/AppInputCluster.vue'
import AppInputHost from '@/components/app/input/AppInputHost.vue'
import DynamicZodForm from '@/components/app/dynamic-zod-form/DynamicZodForm.vue'
import { useEndpointsStore } from '@/stores/endpoints.store'
import { useAuthStore } from '@/stores/auth.store'
import { FormMode } from '@@/shared/types/app.type'
import { CreateEndpointRequest } from '@@/shared/generated/patrolliAPI.schemas.type'
import { endpointHints } from '@/app/services/forms/ui-hints/endpoint.hints'
import type { IEndpointForm } from '@@/shared/types/endpoint.type'

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

const endpointsStore = useEndpointsStore()
const authStore = useAuthStore()
const { loading, endpointFormMode, currentEndpoint } = storeToRefs(endpointsStore)
const { isClusterReadonly } = storeToRefs(authStore)

const dynamicFormRef = ref<InstanceType<typeof DynamicZodForm> | null>(null)
const selectedCluster = ref(appHelper.getDefaultClusterValue())

const form = ref<Record<string, any>>({
	name: '',
	host_id: '',
	adm_status: 1,
	config: {
		type: 'snmp_v1',
		config: {
			address: '',
			community: ''
		}
	}
})

const saveOrUpdate = async (validFormData: IEndpointForm) => {
	let res
	const clusterURLs = [selectedCluster.value]
	if (endpointFormMode.value === FormMode.ADD) {
		res = await endpointsStore.createEndpoint(validFormData, clusterURLs)
	} else {
		res = await endpointsStore.updateEndpoint(
			currentEndpoint.value?.endpoint_id ?? '',
			validFormData,
			clusterURLs
		)
	}
	return res
}

const handleFormSubmit = async (validFormData: any) => {
	loading.value.form = true
	try {
		const res = await saveOrUpdate(validFormData as IEndpointForm)
		if (res) {
			closeFormModal()
			emits('after:submit')
		}
	} catch {
		// Errors handled in store
	} finally {
		loading.value.form = false
	}
}

const triggerSubmit = () => {
	dynamicFormRef.value?.validateAndSubmit()
}

const closeFormModal = () => {
	loading.value.form = false
	endpointsStore.toggleEndpointModalForm(false)
}

const setDefaultFormEdit = () => {
	if (!currentEndpoint.value) return
	const rawConfig = currentEndpoint.value.config
	form.value.name = currentEndpoint.value.name ?? ''
	form.value.host_id = currentEndpoint.value.host_id ?? ''
	form.value.adm_status = Number(currentEndpoint.value.adm_status ?? 1) as 0 | 1
	form.value.config = rawConfig ? JSON.parse(JSON.stringify(rawConfig)) : undefined
}

const isInitializing = ref(true)

onMounted(() => {
	const activeClusterURL = appHelper.getDefaultClusterValue()
	selectedCluster.value = appHelper.getClusterValue(
		currentEndpoint.value?.cluster_name ||
			currentEndpoint.value?.cluster_url ||
			activeClusterURL
	)

	if (endpointFormMode.value === FormMode.UPDATE) {
		setDefaultFormEdit()
	}

	endpointsStore.fetchHosts([selectedCluster.value])

	setTimeout(() => {
		isInitializing.value = false
	}, 0)
})

watch(
	() => currentEndpoint.value,
	() => {
		if (endpointFormMode.value === FormMode.UPDATE && currentEndpoint.value) {
			setDefaultFormEdit()
		}
	},
	{ deep: true }
)

watch(selectedCluster, (newVal, oldVal) => {
	if (!isInitializing.value && newVal !== oldVal) {
		form.value.host_id = ''
	}
})
</script>

<template>
	<div id="endpoint-form" class="space-y-4">
		<div v-if="showTitle" class="flex flex-row gap-3 items-center pb-2">
			<div
				class="flex flex-row justify-center items-center p-2 rounded-lg bg-sky-100 dark:bg-sky-950 text-sky-600 dark:text-sky-400"
			>
				<UiIcon name="api" size="20px" />
			</div>
			<h4 class="text-base font-semibold text-foreground">
				{{ endpointFormMode === FormMode.ADD ? 'Create Endpoint' : 'Edit Endpoint' }}
			</h4>
		</div>

		<div class="flex flex-col gap-2 max-h-[calc(100vh-325px)] overflow-auto p-1">
			<DynamicZodForm
				ref="dynamicFormRef"
				v-model="form"
				:initial-data="endpointFormMode === FormMode.UPDATE ? currentEndpoint : null"
				:schema="CreateEndpointRequest"
				:hints="endpointHints"
				:disabled="loading.form"
				:loading="loading.form"
				@submit="handleFormSubmit"
			>
				<template #field:host_id="{ error, updateModelValue }">
					<div class="flex flex-col gap-2">
						<AppInputCluster
							v-model="selectedCluster"
							label="Cluster"
							required
							:disabled="endpointFormMode === FormMode.UPDATE || isClusterReadonly"
						/>
						<AppInputHost
							v-model="form.host_id"
							@update:model-value="(val: any) => updateModelValue?.(val)"
							label="Host"
							show-label
							required
							:cluster-url="selectedCluster"
							:multiple="false"
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

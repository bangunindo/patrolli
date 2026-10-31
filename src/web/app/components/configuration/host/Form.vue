<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { onMounted, reactive, ref } from 'vue'
import { storeToRefs } from 'pinia'
import { useHostsStore } from '@/stores/hosts.store'
import { useAuthStore } from '@/stores/auth.store'
import { FormMode } from '@@/shared/types/app.type'
import type { IHostForm, IHostFormTemplate } from '@@/shared/types/host.type'
import { CreateHostRequest } from '@@/shared/generated/patrolliAPI.schemas.type'
import { hostHints } from '@/app/services/forms/ui-hints/host.hints'

import appHelper from '@/app/helpers/app.helper'
import AppInputCluster from '@/components/app/input/AppInputCluster.vue'
import AppInputSelect from '@/components/app/input/AppInputSelect.vue'
import AppInputText from '@/components/app/input/AppInputText.vue'
import AppInputLabel from '@/components/app/input/Label.vue'
import ModalUnapplyTemplate from '@/components/configuration/host/ModalUnapplyTemplate.vue'
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

const store = useHostsStore()
const authStore = useAuthStore()
const { isClusterReadonly } = storeToRefs(authStore)
const { hostFormMode, currentHost, loading } = storeToRefs(store)

const dynamicFormRef = ref<InstanceType<typeof DynamicZodForm> | null>(null)
const selectedCluster = ref(appHelper.getDefaultClusterValue())
const form = ref<IHostForm>({
	adm_status: 1,
	labels: JSON.stringify({}),
	name: ''
})

const formTemplate = reactive<IHostFormTemplate>({
	template_id: []
})

const showUnapplyModal = ref(false)
const unlinkedTemplatesToConfirm = ref<Array<{ template_id: string; name: string }>>([])
const pendingFormData = ref<any>(null)

const saveOrUpdate = async (
	validFormData: any,
	unapplyChoices: { template_id: string; keep_items: boolean }[] = []
) => {
	loading.value.form = true
	let res

	if (typeof validFormData.labels === 'string') {
		try {
			validFormData.labels = JSON.parse(validFormData.labels || '{}')
		} catch {
			validFormData.labels = {}
		}
	} else if (!validFormData.labels || typeof validFormData.labels !== 'object') {
		validFormData.labels = {}
	}

	if (hostFormMode.value === FormMode.ADD) {
		res = await store.createHost(validFormData, formTemplate, [selectedCluster.value])
	} else {
		res = await store.updateHost(
			currentHost.value?.host_id ?? '',
			validFormData,
			formTemplate,
			unapplyChoices,
			[selectedCluster.value]
		)
	}
	return res
}

const handleFormSubmit = async (validFormData: any) => {
	if (
		!selectedCluster.value ||
		(Array.isArray(selectedCluster.value) && !selectedCluster.value[0])
	) {
		return
	}
	loading.value.form = true

	if (hostFormMode.value === FormMode.UPDATE) {
		const existingTemplateIds = currentHost.value?.templates?.map((t) => t.template_id) || []
		const newTemplateIds = formTemplate.template_id || []

		const toUnapplyIds = existingTemplateIds.filter((tid) => !newTemplateIds.includes(tid))

		if (toUnapplyIds.length > 0) {
			unlinkedTemplatesToConfirm.value =
				currentHost.value?.templates
					?.filter((t) => toUnapplyIds.includes(t.template_id || ''))
					.map((t) => ({
						template_id: t.template_id || '',
						name: t.name || ''
					})) || []
			pendingFormData.value = { ...validFormData }
			showUnapplyModal.value = true
			loading.value.form = false
			return // Pause submission
		}
	}

	const res = await saveOrUpdate({ ...validFormData })
	if (res) {
		closeFormModal()
		emits('after:submit')
	} else {
		loading.value.form = false
	}
}

const triggerSubmit = () => {
	dynamicFormRef.value?.validateAndSubmit()
}

const onConfirmUnapply = async (unapplyChoices: { template_id: string; keep_items: boolean }[]) => {
	showUnapplyModal.value = false
	loading.value.form = true
	try {
		const res = await saveOrUpdate(pendingFormData.value, unapplyChoices)
		if (res) {
			closeFormModal()
			emits('after:submit')
		} else {
			loading.value.form = false
		}
	} catch {
		loading.value.form = false
	}
}

const closeFormModal = () => {
	loading.value.form = false

	if (props.showCancel) {
		store.toggleHostModalForm(false)
	}
}

onMounted(() => {
	if (hostFormMode.value === FormMode.UPDATE) {
		form.value.labels =
			typeof currentHost.value?.labels === 'object' && currentHost.value?.labels !== null
				? JSON.stringify(currentHost.value?.labels)
				: (currentHost.value?.labels as unknown as string) || JSON.stringify({})
		form.value.name = currentHost.value?.name ?? ''
		form.value.adm_status = currentHost.value?.adm_status ?? 1
	}

	const activeClusterURL = appHelper.getDefaultClusterValue()
	selectedCluster.value = appHelper.getClusterValue(
		currentHost.value?.cluster_name || currentHost.value?.cluster_url || activeClusterURL
	)

	store.fetchTemplates([selectedCluster.value])
})

store.$onAction((context) => {
	if (context.name === 'fetchTemplates') {
		context.after(() => {
			context.store.currentHost?.templates?.forEach((template) => {
				formTemplate.template_id?.push(template.template_id)
			})
		})
	}
}, true)
</script>

<template>
	<div class="host-form-wrapper space-y-4">
		<div v-if="showTitle" class="flex flex-row gap-3 items-center pb-2">
			<div
				class="flex flex-row justify-center items-center p-2 rounded-lg bg-sky-100 dark:bg-sky-950 text-sky-600 dark:text-sky-400"
			>
				<UiIcon name="server" size="20px" />
			</div>
			<h4 class="text-base font-semibold text-foreground">
				{{ hostFormMode === FormMode.ADD ? 'Create Host' : 'Edit Host' }}
			</h4>
		</div>

		<div class="flex flex-col gap-2 max-h-[calc(100vh-325px)] overflow-auto p-1">
			<DynamicZodForm
				ref="dynamicFormRef"
				v-model="form"
				:initial-data="hostFormMode === FormMode.UPDATE ? currentHost : null"
				:schema="CreateHostRequest"
				:hints="hostHints"
				:disabled="loading.form"
				:loading="loading.form"
				@submit="handleFormSubmit"
			>
				<template #field:name="{ error, updateModelValue }">
					<div class="flex flex-col gap-2">
						<AppInputText
							v-model="form.name"
							@update:model-value="(val: any) => updateModelValue?.(val)"
							variant="outline"
							placeholder="Name of hosts"
							label-class="font-semibold"
							label="Name"
							required
							:error="error"
						/>
						<AppInputCluster
							v-model="selectedCluster"
							label="Cluster"
							required
							:disabled="hostFormMode === FormMode.UPDATE || isClusterReadonly"
						/>
					</div>
				</template>

				<template #field:labels="{ error, updateModelValue }">
					<div class="flex flex-col gap-2">
						<AppInputLabel
							v-model="form.labels"
							@update:model-value="(val: any) => updateModelValue?.(val)"
							label="Labels"
							required
							:error="error"
						/>
						<AppInputSelect
							v-model="formTemplate.template_id"
							variant="outline"
							placeholder="Select Templates"
							label-class="font-semibold"
							label="Templates"
							:options="store.selection.templates"
							search
							multiple
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

		<ModalUnapplyTemplate
			:show-modal="showUnapplyModal"
			:unlinked-templates="unlinkedTemplatesToConfirm"
			@confirm="onConfirmUnapply"
			@dismiss="showUnapplyModal = false"
		/>
	</div>
</template>

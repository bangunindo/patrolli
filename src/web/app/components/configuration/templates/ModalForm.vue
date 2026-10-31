<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed, onMounted, reactive, ref } from 'vue'
import { storeToRefs } from 'pinia'
import { useTemplatesStore } from '@/stores/templates.store'
import { useAuthStore } from '@/stores/auth.store'
import { FormMode } from '@@/shared/types/app.type'
import type { ITemplateForm } from '@@/shared/types/template.type'
import { CreateTemplateRequest } from '@@/shared/generated/patrolliAPI.schemas.type'
import { templateHints } from '@/app/services/forms/ui-hints/host.hints'

import appHelper from '@/app/helpers/app.helper'
import AppInputCluster from '@/components/app/input/AppInputCluster.vue'
import AppInputSelect from '@/components/app/input/AppInputSelect.vue'
import AppInputText from '@/components/app/input/AppInputText.vue'
import AppInputLabel from '@/components/app/input/Label.vue'
import ModalUnlinkTemplate from '@/components/configuration/templates/ModalUnlinkTemplate.vue'
import DynamicZodForm from '@/components/app/dynamic-zod-form/DynamicZodForm.vue'

const templatesStore = useTemplatesStore()
const authStore = useAuthStore()

const { templateModalShow, currentTemplate, templateFormMode, loading, templates } =
	storeToRefs(templatesStore)
const { isClusterReadonly } = storeToRefs(authStore)

const dynamicFormRef = ref<InstanceType<typeof DynamicZodForm> | null>(null)
const selectedCluster = ref(appHelper.getDefaultClusterValue())
const form = ref<ITemplateForm>({
	labels: JSON.stringify({}),
	name: ''
})

const formTemplate = reactive({
	template_ids: [] as string[]
})

const showUnlinkModal = ref(false)
const unlinkedTemplatesToConfirm = ref<Array<{ template_id: string; name: string }>>([])
const pendingFormData = ref<{
	form: ITemplateForm
	linkIds: string[]
	unlinkChoices: { unlink_from_template_id: string; keep_items: boolean }[]
} | null>(null)

// Compute templates for selection (excluding current template to prevent self-linking)
const availableTemplates = computed(() => {
	if (templateFormMode.value === FormMode.UPDATE && currentTemplate.value) {
		return templates.value.filter((t) => t.template_id !== currentTemplate.value?.template_id)
	}
	return templates.value
})

const saveOrUpdate = async (
	formData: ITemplateForm,
	linkIds: string[],
	unlinkChoices: { unlink_from_template_id: string; keep_items: boolean }[]
) => {
	try {
		let templateId = ''
		const clusterURLs = [selectedCluster.value]

		let payload: any = { ...formData }
		if (typeof payload.labels === 'string') {
			try {
				payload.labels = JSON.parse(payload.labels || '{}')
			} catch {
				payload.labels = {}
			}
		} else if (!payload.labels || typeof payload.labels !== 'object') {
			payload.labels = {}
		}

		if (templateFormMode.value === FormMode.ADD) {
			const res = await templatesStore.createTemplate(payload, clusterURLs)
			if (res && res.data) {
				templateId = res.data.template_id
			}
		} else {
			templateId = currentTemplate.value?.template_id ?? ''
			await templatesStore.updateTemplate(templateId, payload, clusterURLs)
		}

		if (templateId) {
			if (unlinkChoices.length > 0) {
				await templatesStore.unlinkTemplatesFor(templateId, unlinkChoices, clusterURLs)
			}
			if (linkIds.length > 0) {
				await templatesStore.linkTemplatesFor(templateId, linkIds, clusterURLs)
			}

			await templatesStore.fetchTemplates()
		}

		return true
	} catch (e) {
		console.error('Failed to save template and links', e)
		return false
	}
}

const handleFormSubmit = (validFormData: any) => {
	if (
		!selectedCluster.value ||
		(Array.isArray(selectedCluster.value) && !selectedCluster.value[0])
	) {
		return
	}
	const existingTemplateIds =
		currentTemplate.value?.links_to_templates?.map((t) => t.template_id) || []
	const newTemplateIdsRaw = formTemplate.template_ids || []
	const newTemplateIds = Array.isArray(newTemplateIdsRaw)
		? newTemplateIdsRaw
		: [newTemplateIdsRaw]

	const toLinkIds = newTemplateIds.filter((tid) => !existingTemplateIds.includes(tid))
	const toUnlinkIds = existingTemplateIds.filter((tid) => !newTemplateIds.includes(tid))

	pendingFormData.value = { form: { ...validFormData }, linkIds: toLinkIds, unlinkChoices: [] }

	if (toUnlinkIds.length > 0) {
		unlinkedTemplatesToConfirm.value = existingTemplateIds
			.filter((tid) => toUnlinkIds.includes(tid))
			.map((tid) => {
				const linkedTemplate = currentTemplate.value?.links_to_templates?.find(
					(t) => t.template_id === tid
				)
				return { template_id: tid, name: linkedTemplate?.name || tid }
			})
		showUnlinkModal.value = true
	} else {
		saveOrUpdate(
			pendingFormData.value.form,
			pendingFormData.value.linkIds,
			pendingFormData.value.unlinkChoices
		).then((res) => {
			if (res) {
				closeFormModal()
			} else {
				loading.value.form = false
			}
		})
	}
}

const triggerSubmit = () => {
	dynamicFormRef.value?.validateAndSubmit()
}

const onConfirmUnlink = async (
	unlinkChoices: { unlink_from_template_id: string; keep_items: boolean }[]
) => {
	showUnlinkModal.value = false
	if (pendingFormData.value) {
		pendingFormData.value.unlinkChoices = unlinkChoices
		const res = await saveOrUpdate(
			pendingFormData.value.form,
			pendingFormData.value.linkIds,
			pendingFormData.value.unlinkChoices
		)
		if (res) {
			closeFormModal()
		} else {
			loading.value.form = false
		}
	}
}

const closeFormModal = () => {
	loading.value.form = false
	formTemplate.template_ids = [] // reset multiselect on close
	templatesStore.toggleTemplateModalForm(false)
}

onMounted(() => {
	if (templateFormMode.value === FormMode.UPDATE) {
		form.value.labels =
			typeof currentTemplate.value?.labels === 'object' &&
			currentTemplate.value?.labels !== null
				? JSON.stringify(currentTemplate.value?.labels)
				: (currentTemplate.value?.labels as unknown as string) || JSON.stringify({})
		form.value.name = currentTemplate.value?.name ?? ''

		if (currentTemplate.value?.links_to_templates) {
			formTemplate.template_ids = currentTemplate.value.links_to_templates.map(
				(t) => t.template_id
			)
		} else {
			formTemplate.template_ids = []
		}
	} else {
		formTemplate.template_ids = []
	}

	const activeClusterURL = appHelper.getDefaultClusterValue()
	selectedCluster.value = appHelper.getClusterValue(
		currentTemplate.value?.cluster_name ||
			currentTemplate.value?.cluster_url ||
			activeClusterURL
	)

	templatesStore.fetchTemplates()
})
</script>

<template>
	<UiDialog v-model:open="templateModalShow">
		<UiDialogContent class="max-w-xl p-6 max-h-[85vh] flex flex-col gap-4">
			<UiDialogHeader>
				<div class="flex items-center gap-3">
					<div
						class="flex items-center justify-center p-2 rounded-lg bg-sky-100 text-sky-600 dark:bg-sky-950/50 dark:text-sky-400"
					>
						<UiIcon name="file-document" size="20px" />
					</div>
					<UiDialogTitle class="text-lg font-semibold text-foreground">
						{{
							templateFormMode === FormMode.ADD ? 'Create Template' : 'Edit Template'
						}}
					</UiDialogTitle>
				</div>
			</UiDialogHeader>

			<div class="flex-1 overflow-y-auto py-2">
				<DynamicZodForm
					ref="dynamicFormRef"
					v-model="form"
					:initial-data="templateFormMode === FormMode.UPDATE ? currentTemplate : null"
					:schema="CreateTemplateRequest"
					:hints="templateHints"
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
								placeholder="Names of Template"
								label-class="font-semibold"
								label="Name"
								required
								:error="error"
							/>
							<AppInputCluster
								v-model="selectedCluster"
								label="Cluster"
								required
								:disabled="
									templateFormMode === FormMode.UPDATE || isClusterReadonly
								"
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
								v-model="formTemplate.template_ids"
								:options="
									availableTemplates.map((item) => ({
										label: item.name,
										value: item.template_id
									}))
								"
								label="Link Templates"
								label-class="font-semibold"
								variant="outline"
								placeholder="Select Templates"
								multiple
							/>
						</div>
					</template>
				</DynamicZodForm>
			</div>

			<UiDialogFooter class="flex items-center justify-end gap-2 pt-3 border-t">
				<UiButton
					type="button"
					variant="outline"
					size="sm"
					:disabled="loading.form"
					@click="closeFormModal"
				>
					Cancel
				</UiButton>
				<UiButton type="button" size="sm" :disabled="loading.form" @click="triggerSubmit">
					<UiIcon
						v-if="loading.form"
						name="refresh"
						size="14px"
						class="animate-spin mr-1.5"
					/>
					<span>Save</span>
				</UiButton>
			</UiDialogFooter>
		</UiDialogContent>
	</UiDialog>

	<ModalUnlinkTemplate
		:show-modal="showUnlinkModal"
		:unlinked-templates="unlinkedTemplatesToConfirm"
		@confirm="onConfirmUnlink"
		@dismiss="showUnlinkModal = false"
	/>
</template>

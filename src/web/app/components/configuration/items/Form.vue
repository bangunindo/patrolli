<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { ref, watch } from 'vue'
import { storeToRefs } from 'pinia'
import AppInputCluster from '@/components/app/input/AppInputCluster.vue'
import AppInputEndpoint from '@/components/app/input/AppInputEndpoint.vue'
import AppInputHost from '@/components/app/input/AppInputHost.vue'
import AppInputListener from '@/components/app/input/AppInputListener.vue'
import AppInputSelect from '@/components/app/input/AppInputSelect.vue'
import AppInputText from '@/components/app/input/AppInputText.vue'
import AppInputLabel from '@/components/app/input/Label.vue'
import DynamicZodForm from '@/components/app/dynamic-zod-form/DynamicZodForm.vue'
import AppInputJson from '@/components/app/input/Json.vue'
import { Dummy } from '@@/shared/generated/patrolliAPI.schemas.type'
import { ItemCategory } from '@@/shared/types/item.type'
import { useItemForm } from '@/composables/useItemForm'
import { useAuthStore } from '@/stores/auth.store'

const authStore = useAuthStore()
const { isClusterReadonly } = storeToRefs(authStore)

const emits = defineEmits(['after:submit'])
const props = withDefaults(
	defineProps<{
		showTitle?: boolean
		showCancel?: boolean
		footerFull?: boolean
		isTemplateContext?: boolean
	}>(),
	{
		showTitle: true,
		showCancel: true,
		footerFull: false,
		isTemplateContext: false
	}
)

const dynamicFormRef = ref<InstanceType<typeof DynamicZodForm> | null>(null)
const hostError = ref<string>('')
const endpointError = ref<string>('')
const listenerError = ref<string>('')
// Dots in static slot names are parsed by Vue as directive modifiers, so bind it dynamically
const dummyConfigSlot = 'field:config.dummy.config'

const {
	form,
	category,
	categoryLabel,
	isFieldsRequired,
	canHaveReceiver,
	isEndpointRequiredForType,
	isListenerRequiredForType,
	namePlaceholder,
	itemsFormMode,
	loading,
	selection,
	route,
	FormMode,
	activeSchema,
	activeHints,
	transformFormData,
	handleFormSubmit,
	closeFormModal,
	selectedCluster
} = useItemForm({
	isTemplateContext: props.isTemplateContext,
	emits
})

const handleModelUpdate = (updatedData: Record<string, any>) => {
	if (!updatedData || typeof updatedData !== 'object') return
	const preservedHostId = form.host_id
	const preservedTemplateId = form.template_id
	Object.assign(form, updatedData)
	if (!form.host_id && preservedHostId) {
		form.host_id = preservedHostId
	}
	if (!form.template_id && preservedTemplateId) {
		form.template_id = preservedTemplateId
	}
}

watch(
	() => form.config?.type,
	() => {
		endpointError.value = ''
		listenerError.value = ''
	}
)

const triggerSubmit = () => {
	let hasCustomError = false

	if (!props.isTemplateContext && itemsFormMode.value === FormMode.ADD && !form.host_id) {
		hostError.value = 'Host is required'
		hasCustomError = true
	} else {
		hostError.value = ''
	}

	if (canHaveReceiver.value) {
		if (isEndpointRequiredForType.value && !form.endpoint_id) {
			endpointError.value = 'Endpoint is required'
			hasCustomError = true
		} else {
			endpointError.value = ''
		}

		if (isListenerRequiredForType.value && !form.listener_id) {
			listenerError.value = 'Listener is required'
			hasCustomError = true
		} else {
			listenerError.value = ''
		}
	} else {
		endpointError.value = ''
		listenerError.value = ''
	}

	if (hasCustomError) return

	dynamicFormRef.value?.validateAndSubmit()
}

defineExpose({
	form,
	handleFormSubmit,
	triggerSubmit,
	activeSchema,
	activeHints,
	category,
	isTemplateContext: props.isTemplateContext
})
</script>

<template>
	<form id="item-form" class="space-y-4" @submit.prevent="triggerSubmit" novalidate>
		<div v-if="showTitle" class="flex flex-row gap-3 items-center pb-2">
			<div
				class="flex flex-row justify-center items-center p-2 rounded-lg bg-sky-100 dark:bg-sky-950 text-sky-600 dark:text-sky-400"
			>
				<UiIcon name="database" size="20px" />
			</div>
			<h4 class="text-base font-semibold text-foreground">
				<template v-if="itemsFormMode === FormMode.ADD">
					{{ isTemplateContext ? 'Create Item for Template' : 'Create Item for Host' }}
				</template>
				<template v-else>
					Edit Item
					<span class="text-muted-foreground font-normal">({{ categoryLabel }})</span>
				</template>
			</h4>
		</div>

		<div class="flex flex-col gap-2 max-h-[calc(100vh-325px)] overflow-auto p-1">
			<DynamicZodForm
				ref="dynamicFormRef"
				:model-value="form"
				:schema="activeSchema"
				:hints="activeHints"
				:disabled="loading.form"
				:loading="loading.form"
				:transform="transformFormData"
				@update:model-value="handleModelUpdate"
				@submit="handleFormSubmit"
			>
				<!-- Name and Host/Cluster/Template Selector Slot -->
				<template #field:name="{ error, updateModelValue }">
					<div class="space-y-4">
						<div class="space-y-2">
							<AppInputText
								:model-value="form.name"
								@update:model-value="
									(val: any) => {
										form.name = val
										updateModelValue?.(val)
									}
								"
								variant="outline"
								:placeholder="namePlaceholder"
								label-class="[&_*]:!font-semibold"
								label="Name"
								:required="isFieldsRequired"
								:error="error"
							/>
						</div>

						<!-- Cluster Selection (for Host context) -->
						<div v-if="!isTemplateContext" class="space-y-2">
							<AppInputCluster
								v-model="selectedCluster"
								label="Cluster"
								required
								:disabled="itemsFormMode === FormMode.UPDATE || isClusterReadonly"
							/>
						</div>

						<!-- Host Selection (for Host context) -->
						<div v-if="!isTemplateContext" class="space-y-2">
							<AppInputHost
								v-model="form.host_id"
								label="Host"
								show-label
								required
								:disabled="category === ItemCategory.APPLIED_TEMPLATE"
								:error="hostError"
								:cluster-url="selectedCluster"
								:multiple="false"
								@update:model-value="
									() => {
										if (form.host_id) hostError = ''
									}
								"
							/>
						</div>

						<!-- Template Selection (for Template context when template_id not in route params) -->
						<div
							v-if="isTemplateContext && !route.params.template_id"
							class="space-y-2"
						>
							<div class="flex flex-row flex-wrap gap-2 w-full">
								<div class="flex-1">
									<AppInputSelect
										v-model="form.template_id"
										variant="outline"
										placeholder="Select template"
										label-class="[&_*]:!font-semibold"
										label="Template"
										:options="selection.templates"
										:disabled="itemsFormMode === FormMode.UPDATE"
										required
									/>
								</div>
							</div>
						</div>
					</div>
				</template>

				<!-- Endpoint input: Only for Host Only & Applied Template items with receiver module -->
				<template #field:endpoint_id="{ error, updateModelValue }">
					<div v-if="canHaveReceiver && isEndpointRequiredForType" class="space-y-2">
						<AppInputEndpoint
							:model-value="form.endpoint_id"
							@update:model-value="
								(val: any) => {
									form.endpoint_id = val
									endpointError = ''
									updateModelValue?.(val)
								}
							"
							:multiple="false"
							:host-ids="[form.host_id]"
							:cluster-url="selectedCluster"
							show-label
							required
							:error="endpointError || error"
						/>
					</div>
				</template>

				<!-- Listener input: Only for Host Only & Applied Template items with listener module -->
				<template #field:listener_id="{ error, updateModelValue }">
					<div v-if="canHaveReceiver && isListenerRequiredForType" class="space-y-2">
						<AppInputListener
							:model-value="form.listener_id"
							@update:model-value="
								(val: any) => {
									form.listener_id = val
									listenerError = ''
									updateModelValue?.(val)
								}
							"
							:multiple="false"
							:cluster-url="selectedCluster"
							show-label
							required
							:error="listenerError || error"
						/>
					</div>
				</template>

				<!-- Collector Module Dummy Slot using AppInputJson -->
				<template #[dummyConfigSlot]="{ error, updateModelValue }">
					<div class="space-y-2">
						<ClientOnly>
							<AppInputJson
								v-model="form.config.config"
								:schema="Dummy"
								label="Dummy Config (JSON)"
								height="350px"
								:error="error"
								@update:model-value="
									(val: any) => {
										form.config.config = val
										updateModelValue?.(val)
									}
								"
							/>
						</ClientOnly>
					</div>
				</template>

				<!-- Status Switch Slot -->
				<template #field:adm_status="{ updateModelValue }">
					<div class="space-y-1.5 mb-1">
						<label
							class="text-xs font-semibold text-foreground flex items-center gap-1"
						>
							Status <span class="text-destructive">*</span>
						</label>
						<div class="flex items-center gap-2 pt-1">
							<UiSwitch
								:checked="form.adm_status === 1"
								@update:checked="
									(val: boolean) => {
										const intVal = val ? 1 : 0
										form.adm_status = intVal
										updateModelValue?.(intVal)
									}
								"
							/>
							<span class="text-sm font-medium">
								{{ form.adm_status === 1 ? 'Active' : 'Inactive' }}
							</span>
						</div>
					</div>
				</template>

				<!-- Labels Slot -->
				<template #field:labels="{ updateModelValue }">
					<div class="space-y-2">
						<AppInputLabel
							v-model="form.labels"
							@update:model-value="(val: any) => updateModelValue?.(val)"
							label="Labels"
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
				type="reset"
				@click="closeFormModal"
			>
				Cancel
			</UiButton>
			<UiButton
				size="sm"
				:class="{ 'flex-1 w-full': props.footerFull }"
				:loading="loading.form"
				type="submit"
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
	</form>
</template>

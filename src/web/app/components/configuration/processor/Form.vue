<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { storeToRefs } from 'pinia'
import { watchOnce } from '@vueuse/core'
import { useProcessorStore } from '@/stores/processor.store'
import { useAuthStore } from '@/stores/auth.store'
import { FormMode } from '@@/shared/types/app.type'
import { type IProcessorForm } from '@@/shared/types/processor.type'
import { CreateProcessorRequest } from '@@/shared/generated/patrolliAPI.schemas.type'
import { processorHints } from '@/app/services/forms/ui-hints/processor.hints'
import { validateOplSyntax } from '@/app/helpers/opl.language'

import appHelper from '@/app/helpers/app.helper'
import AppInputCluster from '@/components/app/input/AppInputCluster.vue'
import AppInputItem from '@/components/app/input/AppInputItem.vue'
import AppInputProcessor from '@/components/app/input/AppInputProcessor.vue'
import AppInputText from '@/components/app/input/AppInputText.vue'
import DynamicZodForm from '@/components/app/dynamic-zod-form/DynamicZodForm.vue'

const emits = defineEmits(['cancel', 'after:submit'])
const props = withDefaults(
	defineProps<{
		showTitle?: boolean
		showCancel?: boolean
		footerFull?: boolean
		formInputItems?: string[]
		formInputProcessors?: string[]
		formInputProcessor?: string[]
		formInputPipeline?: string[]
		itemKey?: any
	}>(),
	{
		showTitle: true,
		showCancel: true,
		footerFull: false
	}
)

const processorStore = useProcessorStore()
const authStore = useAuthStore()

const {
	processorFormMode,
	currentProcessor,
	loading,
	customSubmitHandler,
	lockedCluster,
	hideInputItemsAndProcessors
} = storeToRefs(processorStore)
const { isClusterReadonly } = storeToRefs(authStore)

const dynamicFormRef = ref<InstanceType<typeof DynamicZodForm> | null>(null)
const selectedCluster = ref(appHelper.getDefaultClusterValue())

const form = ref<IProcessorForm>({
	adm_status: 1,
	name: '',
	input_items: (props.formInputItems as string[]) ?? [],
	input_processors:
		((props.formInputProcessors ??
			props.formInputProcessor ??
			props.formInputPipeline) as string[]) ?? [],
	config: {
		type: 'null',
		module: 'null',
		config: undefined
	}
})

const saveOrUpdate = async (validFormData: any) => {
	loading.value.form = true
	let res
	const clusterURLs = [selectedCluster.value]

	if (customSubmitHandler.value) {
		res = await customSubmitHandler.value(validFormData)
		return res
	}

	if (processorFormMode.value === FormMode.ADD) {
		res = await processorStore.createProcessor(validFormData, clusterURLs)
	} else {
		res = await processorStore.updateProcessor(
			currentProcessor.value?.processor_id ??
				(currentProcessor.value as any)?.pipeline_id ??
				'',
			validFormData,
			clusterURLs
		)
	}
	return res
}

const handleFormSubmit = async (validFormData: any) => {
	loading.value.form = true
	try {
		// Normalize payload according to Zod discriminated union:
		// For module 'null', config property must not be present
		const procType = validFormData.config?.type ?? validFormData.config?.module ?? 'null'

		if (procType === 'transform') {
			const oplCode = validFormData.config?.config?.opl ?? ''
			const validation = validateOplSyntax(oplCode)
			if (!validation.isValid) {
				if (dynamicFormRef.value) {
					dynamicFormRef.value.issues = {
						'config.config.opl': validation.error || 'Sintaks OPL tidak valid'
					}
				}
				loading.value.form = false
				return
			}
		}

		const payload: any = {
			adm_status: validFormData.adm_status,
			name: validFormData.name,
			input_items: hideInputItemsAndProcessors.value ? [] : validFormData.input_items,
			input_processors: hideInputItemsAndProcessors.value
				? []
				: (validFormData.input_processors ?? validFormData.input_pipelines),
			config: {
				type: procType,
				module: procType
			}
		}

		if (procType !== 'null' && validFormData.config?.config !== undefined) {
			payload.config.config = validFormData.config.config
		}

		const res = await saveOrUpdate(payload)
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

const triggerSubmit = () => {
	dynamicFormRef.value?.validateAndSubmit()
}

const closeFormModal = () => {
	loading.value.form = false

	if (props.showCancel) {
		processorStore.toggleProcessorModalForm(false)
	}

	emits('cancel')
}

watchOnce(
	() => props.formInputItems,
	(val) => {
		form.value.input_items = val ?? []
	}
)

watchOnce(
	() => props.formInputProcessors ?? props.formInputProcessor ?? props.formInputPipeline,
	(val) => {
		form.value.input_processors = val ?? []
	}
)

const setDefaultFormEdit = () => {
	if (!currentProcessor.value) return
	const rawConfig = currentProcessor.value?.config as any
	const procType = rawConfig?.type ?? rawConfig?.module ?? 'null'
	form.value.name = currentProcessor.value?.name ?? ''
	form.value.adm_status = Number(currentProcessor.value?.adm_status ?? 1) as 0 | 1
	form.value.input_processors = [
		...(currentProcessor.value?.input_processors ??
			(currentProcessor.value as any)?.input_pipelines ??
			[])
	].noDuplicate()
	form.value.input_items = [...(currentProcessor.value?.input_items ?? [])].noDuplicate()
	form.value.config = rawConfig
		? {
				type: procType,
				module: procType,
				...(rawConfig.config !== undefined
					? { config: JSON.parse(JSON.stringify(rawConfig.config)) }
					: {})
			}
		: { type: 'null', module: 'null', config: undefined }
}

const formHints = computed(() => {
	if (hideInputItemsAndProcessors.value) {
		return {
			...processorHints,
			input_items: { hidden: true },
			input_processors: { hidden: true },
			input_pipelines: { hidden: true }
		}
	}
	return processorHints
})

const isInitializing = ref(true)

onMounted(async () => {
	loading.value.form = false

	const activeClusterURL = appHelper.getDefaultClusterValue()
	selectedCluster.value = lockedCluster.value
		? appHelper.getClusterValue(lockedCluster.value)
		: appHelper.getClusterValue(
				currentProcessor.value?.cluster_name ||
					currentProcessor.value?.cluster_url ||
					activeClusterURL
			)

	if (processorFormMode.value === FormMode.UPDATE) {
		setDefaultFormEdit()
	}

	setTimeout(() => {
		isInitializing.value = false
	}, 0)
})

watch(
	lockedCluster,
	(val) => {
		if (val) {
			selectedCluster.value = appHelper.getClusterValue(val)
		}
	},
	{ immediate: true }
)

watch(
	() => currentProcessor.value,
	() => {
		if (processorFormMode.value === FormMode.UPDATE && currentProcessor.value) {
			setDefaultFormEdit()
		}
	},
	{ deep: true }
)

watch(selectedCluster, (newVal, oldVal) => {
	if (!isInitializing.value && newVal !== oldVal) {
		form.value.input_items = []
		form.value.input_processors = []
	}
})
</script>

<template>
	<div id="processor-form" class="space-y-4">
		<div v-if="showTitle" class="flex flex-row gap-3 items-center pb-2">
			<div
				class="flex flex-row justify-center items-center p-2 rounded-lg bg-sky-100 dark:bg-sky-950 text-sky-600 dark:text-sky-400"
			>
				<UiIcon name="transit-connection-variant" size="20px" />
			</div>
			<h4 class="text-base font-semibold text-foreground">
				{{ processorFormMode === FormMode.ADD ? 'Create Processor' : 'Edit Processor' }}
			</h4>
		</div>

		<div class="flex flex-col gap-2 max-h-[calc(100vh-325px)] overflow-auto p-1 pb-6">
			<DynamicZodForm
				ref="dynamicFormRef"
				v-model="form"
				:initial-data="processorFormMode === FormMode.UPDATE ? currentProcessor : null"
				:schema="CreateProcessorRequest"
				:hints="formHints"
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
							placeholder="Name of Processor"
							label="Name"
							label-class="font-semibold"
							required
							:error="error"
						/>
						<AppInputCluster
							v-model="selectedCluster"
							label="Cluster"
							required
							:disabled="
								processorFormMode === FormMode.UPDATE ||
								isClusterReadonly ||
								!!lockedCluster
							"
						/>
					</div>
				</template>

				<template #field:input_items v-if="!hideInputItemsAndProcessors">
					<div class="flex flex-col gap-2">
						<AppInputItem
							v-model="form.input_items"
							show-label
							:cluster-url="selectedCluster"
						/>
					</div>
				</template>

				<template #field:input_processors v-if="!hideInputItemsAndProcessors">
					<div class="flex flex-col gap-2">
						<AppInputProcessor
							v-model="form.input_processors"
							show-label
							:cluster-url="selectedCluster"
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

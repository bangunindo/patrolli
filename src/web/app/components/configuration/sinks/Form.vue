<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { onMounted, ref, watch } from 'vue'
import { storeToRefs } from 'pinia'
import { useSinksStore } from '@/stores/sinks.store'
import { useAuthStore } from '@/stores/auth.store'
import { FormMode } from '@@/shared/types/app.type'
import { CreateSinkRequest } from '@@/shared/generated/patrolliAPI.schemas.type'
import { sinkHints } from '@/app/services/forms/ui-hints/sink.hints'
import { SinkChanelCompression, SinkChanelType, SinkConfigType } from '@@/shared/types/sink.type'

import appHelper from '@/app/helpers/app.helper'
import AppInputCluster from '@/components/app/input/AppInputCluster.vue'
import AppInputProcessor from '@/components/app/input/AppInputProcessor.vue'
import AppInputText from '@/components/app/input/AppInputText.vue'
import AppInputLabel from '@/components/app/input/Label.vue'
import DynamicZodForm from '@/components/app/dynamic-zod-form/DynamicZodForm.vue'

const emits = defineEmits(['cancel', 'after:submit'])
const props = withDefaults(
	defineProps<{
		showTitle?: boolean
		showCancel?: boolean
		footerFull?: boolean
		showInputs?: boolean
		formInputs?: string[]
	}>(),
	{
		showTitle: true,
		showCancel: true,
		footerFull: false,
		showInputs: true
	}
)

const sinksStore = useSinksStore()
const authStore = useAuthStore()

const { loading, sinkFormMode, currentSink } = storeToRefs(sinksStore)
const { isClusterReadonly } = storeToRefs(authStore)

const dynamicFormRef = ref<InstanceType<typeof DynamicZodForm> | null>(null)
const selectedCluster = ref(appHelper.getDefaultClusterValue())
const form = ref<Record<string, any>>({
	adm_status: 1,
	name: '',
	inputs: [] as any[],
	labels: JSON.stringify({}),
	worker_count: 1,
	batch: {
		batch_size: 1000,
		batch_timeout: 1000
	} as any,
	channel: {
		compression: SinkChanelCompression.NONE,
		max_disk_size: 0,
		max_memtable_size: 0,
		non_blocking: false,
		type: SinkChanelType.DISK
	} as any,
	config: {
		type: SinkConfigType.FILE,
		config: undefined
	} as any
})

const saveOrUpdate = async (validFormData: any) => {
	loading.value.form = true
	let res

	if (validFormData.batch && form.value.batch?.batch_timeout) {
		validFormData.batch.batch_timeout = parseInt(form.value.batch.batch_timeout)
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

	if (sinkFormMode.value === FormMode.ADD) {
		res = await sinksStore.createSink(validFormData, clusterURLs)
	} else {
		const id = currentSink.value?.sink_id ?? ''
		res = await sinksStore.updateSink(id, validFormData, clusterURLs)
	}

	return res
}

const handleFormSubmit = async (validFormData: any) => {
	loading.value.form = true
	try {
		const res = await saveOrUpdate(validFormData)
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
		sinksStore.toggleSinkModalForm(false)
	}

	emits('cancel')
}

const setDefaultFormEdit = (inputs: string[] = []) => {
	if (!currentSink.value) return
	const rawBatch = currentSink.value?.batch
	const rawChannel = currentSink.value?.channel
	const rawConfig = currentSink.value?.config
	form.value.adm_status = Number(currentSink.value?.adm_status ?? 1)
	form.value.name = currentSink.value?.name ?? ''
	form.value.batch = rawBatch ? JSON.parse(JSON.stringify(rawBatch)) : undefined
	form.value.channel = rawChannel ? JSON.parse(JSON.stringify(rawChannel)) : undefined
	form.value.labels = (currentSink.value?.labels as unknown as string) ?? JSON.stringify({})
	form.value.worker_count = currentSink.value?.worker_count || 1
	form.value.inputs = [...(currentSink.value?.inputs ?? []), ...inputs].noDuplicate()
	form.value.config = rawConfig ? JSON.parse(JSON.stringify(rawConfig)) : undefined

	const activeClusterURL = appHelper.getDefaultClusterValue()
	selectedCluster.value = appHelper.getClusterValue(
		currentSink.value?.cluster_name || currentSink.value?.cluster_url || activeClusterURL
	)
}

const isInitializing = ref(true)

onMounted(async () => {
	const inputs: string[] = props.formInputs ?? []

	const activeClusterURL = appHelper.getDefaultClusterValue()
	selectedCluster.value = appHelper.getClusterValue(
		currentSink.value?.cluster_name || currentSink.value?.cluster_url || activeClusterURL
	)

	if (sinkFormMode.value === FormMode.UPDATE) {
		if (props.formInputs === undefined) {
			if (currentSink.value?.sink_id) {
				const clusterUrl = currentSink.value?.cluster_url
				await sinksStore.fetchSinkById(
					currentSink.value.sink_id,
					clusterUrl ? [clusterUrl] : undefined
				)
			}
		} else {
			setDefaultFormEdit(inputs)
		}
	} else {
		form.value.inputs = inputs
	}

	setTimeout(() => {
		isInitializing.value = false
	}, 0)
})

sinksStore.$onAction((context) => {
	context.after(async () => {
		if (context.name === 'fetchSinkById') {
			setDefaultFormEdit(sinksStore.currentSink?.inputs)
		}
	})
})

watch(selectedCluster, (newVal, oldVal) => {
	if (!isInitializing.value && newVal !== oldVal) {
		form.value.inputs = []
	}
})

// Auto-check/uncheck SSL & SASL based on Kafka security protocol
watch(
	() => {
		const cfg = form.value?.config
		if (cfg?.type === 'kafka') {
			return cfg?.config?.security?.protocol
		}
		return undefined
	},
	(newProtocol, oldProtocol) => {
		if (isInitializing.value || !newProtocol || newProtocol === oldProtocol) return

		const kafkaConfig = form.value?.config?.config
		if (!kafkaConfig) return

		if (newProtocol === 'plaintext') {
			kafkaConfig.ssl = null
			kafkaConfig.sasl = null
		} else if (newProtocol === 'ssl') {
			kafkaConfig.ssl = kafkaConfig.ssl || {
				ca_cert_path: '',
				cert_path: '',
				key_path: '',
				key_password: '',
				insecure_skip_verify: false
			}
			kafkaConfig.sasl = null
		} else if (newProtocol === 'sasl_plaintext') {
			kafkaConfig.ssl = null
			kafkaConfig.sasl = kafkaConfig.sasl || {
				mechanism: 'plain',
				username: '',
				password: ''
			}
		} else if (newProtocol === 'sasl_ssl') {
			kafkaConfig.ssl = kafkaConfig.ssl || {
				ca_cert_path: '',
				cert_path: '',
				key_path: '',
				key_password: '',
				insecure_skip_verify: false
			}
			kafkaConfig.sasl = kafkaConfig.sasl || {
				mechanism: 'plain',
				username: '',
				password: ''
			}
		}
	}
)
</script>

<template>
	<div id="sink-form" class="space-y-4">
		<div v-if="showTitle" class="flex flex-row gap-3 items-center pb-2">
			<div
				class="flex flex-row justify-center items-center p-2 rounded-lg bg-sky-100 dark:bg-sky-950 text-sky-600 dark:text-sky-400"
			>
				<UiIcon name="server" size="20px" />
			</div>
			<h4 class="text-base font-semibold text-foreground">
				{{ sinkFormMode === FormMode.ADD ? 'Create Sink' : 'Edit Sink' }}
			</h4>
		</div>

		<div class="flex flex-col gap-2 max-h-[calc(100vh-325px)] overflow-auto p-1">
			<DynamicZodForm
				ref="dynamicFormRef"
				v-model="form"
				:initial-data="sinkFormMode === FormMode.UPDATE ? currentSink : null"
				:schema="CreateSinkRequest"
				:hints="sinkHints"
				:disabled="loading.form"
				:loading="loading.form"
				@submit="handleFormSubmit"
			>
				<template #field:name="{ error, updateModelValue }">
					<div class="flex flex-col gap-2">
						<AppInputText
							v-model="form.name"
							@update:model-value="(val: any) => updateModelValue?.(val)"
							placeholder="Name of group"
							label="Name"
							required
							:error="error"
						/>
						<AppInputCluster
							v-model="selectedCluster"
							label="Cluster"
							required
							:disabled="sinkFormMode === FormMode.UPDATE || isClusterReadonly"
						/>
					</div>
				</template>

				<template #field:labels="{ updateModelValue }">
					<div class="flex flex-col gap-2">
						<AppInputLabel
							v-model="form.labels"
							@update:model-value="(val: any) => updateModelValue?.(val)"
							label="Labels"
							required
						/>
					</div>
				</template>

				<template #field:inputs="{ updateModelValue }">
					<div class="flex flex-col gap-2">
						<AppInputProcessor
							v-model="form.inputs"
							@update:model-value="(val: any) => updateModelValue?.(val)"
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

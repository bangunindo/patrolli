<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { AppWebSocket } from '@/app/services/websocket.service'
import AppInputText from '@/components/app/input/AppInputText.vue'
import AppLoading from '@/components/app/loading/AppLoading.vue'
import AppTable from '@/components/app/AppTable.vue'
import { useAppStore } from '@/stores/app.store'
import { useProcessorStore } from '@/stores/processor.store'
import { storeToRefs } from 'pinia'
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import appHelper from '@/app/helpers/app.helper'
import ProcessorForm from '@/components/configuration/processor/Form.vue'

const headerTableIn = [
	{
		key: 'index',
		title: '#'
	},
	{
		key: 'message_in',
		title: 'Data In'
	}
]

const headerTableOut = [
	{
		key: 'index',
		title: '#'
	},
	{
		key: 'message_out',
		title: 'Data Out'
	}
]

const appStore = useAppStore()
const store = useProcessorStore()
const { currentProcessor } = storeToRefs(store)
const { activeClusterURL } = storeToRefs(appStore)

const limit = ref(10)

const path = ref<string>('')
let ws: AppWebSocket | undefined
const dataProcessor = ref<any[]>([])
const dataDetail = ref<any>()
const dataDetailModal = ref<boolean>(false)
const loading = ref<boolean>(true)
const type = ref<string>('message_in')
let noDataTimer: ReturnType<typeof setTimeout> | undefined

const startNoDataTimer = () => {
	clearNoDataTimer()

	noDataTimer = setTimeout(() => {
		if (dataProcessor.value.length === 0) {
			ws?.close()
		}
	}, 60000)
}

const clearNoDataTimer = () => {
	if (noDataTimer) {
		clearTimeout(noDataTimer)
		noDataTimer = undefined
	}
}

const handleWebSocketError = (data: any) => {
	const title = data?.error?.title ?? 'Failed to start capture'
	const detail =
		data?.error?.detail ??
		(typeof data?.error === 'string' ? data?.error : 'Error starting capture')

	appStore.showError(title, detail)
	ws?.close()
	loading.value = false
}

const initWebSocket = () => {
	ws?.close()
	clearNoDataTimer()

	const rawBaseUrl = (activeClusterURL.value || appHelper.getDefaultClusterValue() || '').replace(
		/\/+$/,
		''
	)
	const cleanBase =
		rawBaseUrl.startsWith('/') || rawBaseUrl.startsWith('http') || rawBaseUrl.startsWith('ws')
			? rawBaseUrl
			: `/${rawBaseUrl}`
	path.value =
		`${cleanBase}/ws/processors/subscribe_capture/${currentProcessor.value?.processor_id ?? (currentProcessor.value as any)?.pipeline_id}`.replace(
			/^\/\//,
			'/'
		)
	ws = new AppWebSocket(path.value)

	ws.addEventListener('open', () => {
		loading.value = true

		if (dataProcessor.value.length === 0) {
			startNoDataTimer()
		}
	})

	ws.addEventListener('message', (e) => {
		try {
			const data: any = JSON.parse(e.data)
			console.log('[Processor Preview Message]:', data)

			if (data?.error || data?.success === false) {
				handleWebSocketError(data)
				return
			}

			const idx = dataProcessor.value.findIndex((v: any) => v?.index == data.index)
			if (idx == -1) {
				dataProcessor.value.push(data)
			}

			if (dataProcessor.value.length > 0) {
				clearNoDataTimer()
			}
		} catch (err) {
			console.error('Failed to parse websocket message:', err)
		}
	})

	ws.addEventListener('close', () => {
		loading.value = false
		clearNoDataTimer()
	})

	ws.addEventListener('error', () => {
		ws?.close()
	})
}

const onRowClick = (data, key = 'message_in') => {
	type.value = key
	dataDetail.value = data
	dataDetailModal.value = true
}

const onAfterSubmit = () => {
	dataProcessor.value = []
	initWebSocket()
}

const stop = () => {
	ws?.close()
}

const reload = () => {
	dataProcessor.value = []
	initWebSocket()
}

watch(
	() => dataProcessor.value.length,
	(val) => {
		if (val == limit.value) {
			ws?.close()
		}
	}
)

watch(
	() => dataDetailModal.value,
	(val) => {
		if (!val) {
			dataDetail.value = undefined
		}
	}
)

watch(
	() => activeClusterURL.value,
	() => {
		dataProcessor.value = []
		initWebSocket()
	}
)

const showdata = computed(() => {
	const data = JSON.parse(JSON.stringify(dataProcessor.value))
	return data
})

onMounted(() => {
	initWebSocket()
})

onUnmounted(() => {
	clearNoDataTimer()
	ws?.close()
	ws = undefined
})
</script>

<template>
	<div class="rounded-lg border bg-card text-card-foreground shadow-sm">
		<div class="p-4 border-b flex justify-between items-center">
			<div class="flex flex-row gap-3 items-center">
				<div
					class="flex flex-row justify-center items-center p-1.5 rounded bg-sky-100 dark:bg-sky-950/50"
				>
					<UiIcon name="cog" class="w-5 h-5 text-sky-600" />
				</div>
				<h3 class="text-base font-semibold leading-tight">Preview Data Processor</h3>
			</div>

			<section class="flex gap-4 justify-center items-center pr-10">
				<div class="flex items-center gap-2">
					<label class="font-semibold text-sm">Stop After</label>
					<AppInputText
						v-model="limit"
						placeholder="Stop After"
						type="number"
						min="1"
						:disabled="loading"
					></AppInputText>
				</div>

				<div class="flex gap-2">
					<UiButton
						variant="outline"
						size="icon"
						class="text-rose-500 hover:text-rose-600"
						:disabled="!loading"
						@click="stop"
					>
						<UiIcon name="stop" class="w-4 h-4" />
					</UiButton>
					<UiButton
						variant="outline"
						size="icon"
						class="text-amber-500 hover:text-amber-600"
						:disabled="loading"
						@click="reload"
					>
						<UiIcon name="reload" class="w-4 h-4" />
					</UiButton>
				</div>
			</section>
		</div>

		<section style="min-height: calc(100vh - 210px)" class="px-3 -mt-[12px]">
			<section class="flex gap-4">
				<div class="flex-1 pt-5">
					<div style="max-height: calc(100vh - 210px)" class="overflow-y-auto">
						<div style="min-height: calc(100vh - 210px)">
							<AppTable
								no-pagination
								v-bind="{
									columns: headerTableIn,
									useAction: true,
									data: showdata,
									loading: store.loading.table
								}"
							>
								<template #col.index="{ data }">
									<div>{{ data.index }}</div>
								</template>

								<template #col.message_in="{ data }">
									<div
										class="truncate"
										style="max-width: calc((100vw / 3) - 175px)"
									>
										{{ data.message_out ?? '-' }}
									</div>
								</template>

								<template #col.action="{ data }">
									<div class="flex justify-end">
										<UiButton
											variant="ghost"
											size="icon"
											class="h-7 w-7 text-primary"
											@click="onRowClick(data, 'message_in')"
										>
											<UiIcon name="eye" class="w-4 h-4" />
										</UiButton>
									</div>
								</template>

								<template #no-data v-if="loading">
									<AppLoading container-height="32px" size="24px"></AppLoading>
								</template>
							</AppTable>
							<AppLoading
								v-if="loading && dataProcessor.length > 0"
								container-height="32px"
								size="24px"
							></AppLoading>
						</div>
					</div>
				</div>

				<div class="flex-1">
					<ProcessorForm
						:key="
							store.currentProcessor?.processor_id ??
							(store.currentProcessor as any)?.pipeline_id
						"
						:show-title="true"
						:show-cancel="false"
						@after:submit="onAfterSubmit"
					></ProcessorForm>
				</div>

				<div class="flex-1 pt-5">
					<div style="max-height: calc(100vh - 210px)" class="overflow-y-auto">
						<div style="min-height: calc(100vh - 210px)">
							<AppTable
								no-pagination
								v-bind="{
									columns: headerTableOut,
									useAction: true,
									data: showdata,
									loading: store.loading.table
								}"
							>
								<template #col.index="{ data }">
									<div>{{ data.index }}</div>
								</template>

								<template #col.message_out="{ data }">
									<div
										class="truncate"
										style="max-width: calc((100vw / 3) - 175px)"
									>
										{{ data.message_out ?? '-' }}
									</div>
								</template>

								<template #col.action="{ data }">
									<div class="flex justify-end">
										<UiButton
											variant="ghost"
											size="icon"
											class="h-7 w-7 text-primary"
											@click="onRowClick(data, 'message_out')"
										>
											<UiIcon name="eye" class="w-4 h-4" />
										</UiButton>
									</div>
								</template>

								<template #no-data v-if="loading">
									<AppLoading container-height="32px" size="24px"></AppLoading>
								</template>
							</AppTable>
							<AppLoading
								v-if="loading && dataProcessor.length > 0"
								container-height="32px"
								size="24px"
							></AppLoading>
						</div>
					</div>
				</div>
			</section>
		</section>
	</div>

	<UiDialog v-model:open="dataDetailModal">
		<UiDialogContent class="max-w-4xl max-h-[85vh] p-6 flex flex-col gap-4">
			<UiDialogHeader>
				<div class="flex items-center gap-3">
					<div
						class="flex items-center justify-center p-2 rounded-lg bg-sky-100 text-sky-600 dark:bg-sky-950/50 dark:text-sky-400"
					>
						<UiIcon name="eye" size="20px" />
					</div>
					<UiDialogTitle class="text-lg font-semibold text-foreground">
						Preview Data Processor - Message In Out
					</UiDialogTitle>
				</div>
			</UiDialogHeader>

			<div class="flex-1 overflow-y-auto max-h-[calc(100vh-250px)]">
				<section class="min-h-[400px] flex gap-4">
					<div class="flex-1 flex flex-col gap-2">
						<span class="font-semibold text-sm text-foreground">Message In</span>
						<json-viewer
							:value="dataDetail?.message_in"
							:expand-depth="10"
							copyable
							sort
						></json-viewer>
					</div>
					<div class="w-px bg-border"></div>
					<div class="flex-1 flex flex-col gap-2">
						<span class="font-semibold text-sm text-foreground">Message Out</span>
						<json-viewer
							:value="dataDetail?.message_out"
							:expand-depth="10"
							copyable
							sort
						></json-viewer>
					</div>
				</section>
			</div>

			<UiDialogFooter class="flex items-center justify-end pt-3 border-t">
				<UiButton variant="outline" size="sm" @click="dataDetailModal = false">
					Close
				</UiButton>
			</UiDialogFooter>
		</UiDialogContent>
	</UiDialog>
</template>

<style>
.jv-container .jv-code {
	padding: 0;
}

.jv-container .jv-node:first-child {
	margin: 0 !important;
}
</style>

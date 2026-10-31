<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import sinksApi from '@/app/api/sinks.api'
import type { ISinkListParams, ISinks } from '@@/shared/types/sink.type'
import { computed, onUnmounted, reactive, ref, watch } from 'vue'
import AppInputSelectTable from '@/components/app/input/AppInputSelectTable.vue'

const props = withDefaults(
	defineProps<{
		headers?: any[]
		required?: boolean
		multiple?: boolean
		label?: string
		showLabel?: boolean
		clusterUrl?: string
		error?: string | string[]
		rules?: string[]
		sinkIds?: string[]
	}>(),
	{
		required: false,
		multiple: true,
		error: '',
		rules: () => [],
		headers: () => [
			{
				key: 'name',
				title: 'Name',
				sortable: true
			},
			{
				key: 'adm_status',
				title: 'Status'
			},
			{
				key: 'type',
				title: 'Type'
			}
		]
	}
)

const emits = defineEmits<{
	close: []
	'update:error': [value: string]
}>()

const modelValue = defineModel<string[] | string>()
const sinks = ref<ISinks[]>([])
const loading = ref<boolean>(true)

const clusterURLs = computed(() => (props.clusterUrl ? [props.clusterUrl] : undefined))

const pagination = reactive({
	rows: 10,
	currentPage: 1,
	totalRecords: 0,
	reachEnd: false,
	search: false
})

const filter = reactive<ISinkListParams>({
	name: undefined,
	order_by: undefined,
	order_dir: undefined,
	limit: pagination.rows,
	offset: (pagination.currentPage - 1) * pagination.rows
})
const errors = ref<string | null>(null)

const loadMore = async (payload: any) => {
	pagination.currentPage = payload.page ?? payload.currentPage
	filter.offset = payload.offset ?? 0

	if (payload.refetch) {
		await fetchSelectionData()
		return
	}

	await fetchData()
}

const onSearch = async (search: string) => {
	pagination.search = true
	pagination.currentPage = 1
	pagination.reachEnd = false

	filter.offset = 0
	filter.name = search

	sinks.value = []

	await fetchData()
	pagination.search = false
}

const fetchSelectionData = async () => {
	const selectedIds = Array.isArray(modelValue.value)
		? modelValue.value
		: modelValue.value
			? [modelValue.value]
			: []

	const response = await sinksApi.getSinks({ sink_ids: selectedIds }, clusterURLs.value)
	if (response?.success && response.data?.sinks) {
		sinks.value = [...response.data.sinks].noDuplicateObject('sink_id')
	}
}

const fetchData = async () => {
	loading.value = true
	errors.value = null

	try {
		if (pagination.reachEnd) return

		if (props.sinkIds && props.sinkIds.length === 0) {
			sinks.value = []
			pagination.totalRecords = 0
			pagination.reachEnd = true
			return
		}

		const queryFilter: ISinkListParams = { ...filter }
		if (props.sinkIds && props.sinkIds.length > 0) {
			queryFilter.sink_ids = props.sinkIds
		}

		const resp = sinksApi.getSinks(queryFilter, clusterURLs.value)
		const promises: Promise<any>[] = [resp]

		if (!pagination.search) {
			const selectedIds = Array.isArray(modelValue.value)
				? modelValue.value
				: modelValue.value
					? [modelValue.value]
					: []

			if (sinks.value.length === 0 && selectedIds.length > 0) {
				const selectedResp = sinksApi.getSinks({ sink_ids: selectedIds }, clusterURLs.value)
				promises.push(selectedResp)
			}
		}

		const responsePromises = await Promise.all(promises)
		const [response, selectedResp] = responsePromises

		if (response?.success) {
			if (selectedResp?.success && selectedResp.data?.sinks) {
				sinks.value = [...selectedResp.data.sinks]
			}

			sinks.value = [...sinks.value, ...(response.data?.sinks || [])].noDuplicateObject(
				'sink_id'
			)

			if (!pagination.search && pagination.currentPage === 1) {
				pagination.totalRecords = response.data?.total || 0
			}

			if (sinks.value.length >= pagination.totalRecords) {
				pagination.reachEnd = true
			}
		}
	} catch (err: any) {
		errors.value = err.message || 'Failed to fetch sinks'
	} finally {
		pagination.search = false
		loading.value = false
	}
}

const showData = computed<any[]>(() => {
	if (props.sinkIds) {
		const allowed = new Set(props.sinkIds)
		return sinks.value.filter((s) => allowed.has(s.sink_id))
	}
	return sinks.value as any[]
})

watch(
	() => props.clusterUrl,
	async () => {
		pagination.currentPage = 1
		pagination.reachEnd = false
		pagination.totalRecords = 0
		filter.offset = 0
		sinks.value = []
		await fetchData()
	}
)

watch(
	() => props.sinkIds,
	async () => {
		pagination.currentPage = 1
		pagination.reachEnd = false
		pagination.totalRecords = 0
		filter.offset = 0
		sinks.value = []
		await fetchData()
	},
	{ deep: true }
)

watch(
	() => modelValue.value,
	async (newVal) => {
		if (!newVal) return
		const selectedIds = Array.isArray(newVal) ? newVal : [newVal]
		const existingIds = new Set(sinks.value.map((s) => s.sink_id))
		const missingIds = selectedIds.filter((id) => id && !existingIds.has(id))
		if (missingIds.length > 0) {
			const res = await sinksApi.getSinks({ sink_ids: missingIds }, clusterURLs.value)
			if (res?.success && res.data?.sinks) {
				sinks.value = [...res.data.sinks, ...sinks.value].noDuplicateObject('sink_id')
			}
		}
	}
)

onUnmounted(() => {
	pagination.currentPage = 1
	pagination.reachEnd = false
	pagination.totalRecords = 0
	filter.offset = 0
})
</script>

<template>
	<div>
		<AppInputSelectTable
			v-model="modelValue"
			class="min-w-[250px] flex-1 flex-grow"
			:label="props.label || 'Sink'"
			value-key="sink_id"
			checkbox-key="name"
			:show-label="props.showLabel"
			:data="showData"
			:pagination="pagination"
			:loading="loading"
			:fetch-on-mount="true"
			:header-table="props.headers"
			:required="props.required"
			:multiple="props.multiple"
			:error="props.error"
			:rules="props.rules"
			@infinite="loadMore"
			@search="onSearch"
			@close="emits('close')"
			@update:error="emits('update:error', $event)"
		/>
	</div>
</template>

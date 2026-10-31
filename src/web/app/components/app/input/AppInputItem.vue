<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import itemsApi from '@/app/api/items.api'
import type { IItem, IItemListParams } from '@@/shared/types/item.type'
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
	}>(),
	{
		required: false,
		multiple: true,
		error: '',
		rules: () => [],
		headers: () => {
			return [
				{
					key: 'resolved_name',
					title: 'Name'
				},
				{
					key: 'adm_status',
					title: 'Status'
				}
			]
		}
	}
)

const emits = defineEmits<{
	close: []
	'update:error': [value: string]
}>()

const modelValue = defineModel<string[] | string>()
const items = ref<IItem[]>([])
const loading = ref<boolean>(true)

const clusterURLs = computed(() => (props.clusterUrl ? [props.clusterUrl] : undefined))

const pagination = reactive({
	rows: 10,
	currentPage: 1,
	totalRecords: 0,
	reachEnd: false,
	search: false
})

const filter = reactive<IItemListParams>({
	name: undefined,
	order_by: undefined,
	order_dir: undefined,
	limit: pagination.rows,
	offset: (pagination.currentPage - 1) * pagination.rows,
	exclusivity: ['host', 'template_on_host']
})
const errors = ref<string | null>(null)

const loadMore = async (payload) => {
	pagination.currentPage = payload.page ?? payload.currentPage
	filter.offset = payload.offset ?? 0

	if (payload.refetch) {
		await fetchSelectionData()
		return
	}

	await fetchData()
}

const onSearch = async (search) => {
	pagination.search = true
	pagination.currentPage = 1
	pagination.reachEnd = false

	filter.offset = 0
	filter.name = search

	items.value = []

	await fetchData()
	pagination.search = false
}

const fetchSelectionData = async () => {
	const selectedIds = Array.isArray(modelValue.value)
		? modelValue.value
		: modelValue.value
			? [modelValue.value]
			: []

	const response = await itemsApi.getItems({ item_ids: selectedIds }, clusterURLs.value)
	if (response?.success) {
		items.value = [...response.data.items].noDuplicateObject('item_id')
	}
}

const fetchData = async () => {
	loading.value = true
	errors.value = null

	try {
		if (pagination.reachEnd) return

		const resp = itemsApi.getItems(filter, clusterURLs.value)
		const promises: Promise<any>[] = [resp]

		if (!pagination.search) {
			const selectedIds = Array.isArray(modelValue.value)
				? modelValue.value
				: modelValue.value
					? [modelValue.value]
					: []

			if (items.value.length === 0 && selectedIds.length > 0) {
				const selectedResp = itemsApi.getItems({ item_ids: selectedIds }, clusterURLs.value)
				promises.push(selectedResp)
			}
		}

		const responsePromises = await Promise.all(promises)

		const [response, selectedResp] = responsePromises

		if (response?.success) {
			if (selectedResp?.success) {
				items.value = [...selectedResp.data.items]
			}

			items.value = [...items.value, ...response.data.items].noDuplicateObject('item_id')

			if (!pagination.search && pagination.currentPage == 1) {
				pagination.totalRecords = response.data.total
			}

			if (items.value.length >= pagination.totalRecords) {
				pagination.reachEnd = true
			}
		}
	} catch (err: any) {
		errors.value = err.message || 'Failed to fetch items'
	} finally {
		pagination.search = false
		loading.value = false
	}
}

const showData = computed<any[]>(() => {
	return items.value as any[]
})

watch(
	() => props.clusterUrl,
	async () => {
		pagination.currentPage = 1
		pagination.reachEnd = false
		pagination.totalRecords = 0
		filter.offset = 0
		items.value = []
		await fetchData()
	}
)

onUnmounted(() => {
	pagination.currentPage = 1
	pagination.reachEnd = false
	pagination.totalRecords = 0
	filter.offset = 0

	modelValue.value = props.multiple ? [] : undefined
})
</script>

<template>
	<div>
		<AppInputSelectTable
			v-model="modelValue"
			class="min-w-[250px] flex-1 flex-grow"
			:label="props.label || 'Items'"
			value-key="item_id"
			checkbox-key="resolved_name"
			:show-label="props.showLabel"
			:data="showData"
			:pagination="pagination"
			:loading="loading"
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

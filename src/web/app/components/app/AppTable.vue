<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed, ref, unref } from 'vue'
import AppLoading from '@/components/app/loading/AppLoading.vue'

type TAlign = 'left' | 'center' | 'right'
type TLoadingPosition = 'top' | 'bottom'
type TSortOrder = 'asc' | 'desc' | null

interface ITableColumn {
	key: string
	title?: string
	sortable?: boolean
	width?: number | string
	align?: TAlign
}

const selected = defineModel<any[]>('selected', { default: () => [] })

// 1. Add emit 'update:sort'
const emits = defineEmits(['update:page', 'update:sort'])

const props = withDefaults(
	defineProps<{
		columns: ITableColumn[]
		data?: any[]
		datas?: any[]
		loading?: boolean
		pagination?: any
		noPagination?: boolean
		loadingPosition?: TLoadingPosition
		useAction?: boolean
		useSelection?: boolean
		selectionKey?: string
		entityLabel?: string
		hidePaginationInfo?: boolean
	}>(),
	{
		columns: () => [
			{
				key: 'index',
				title: '#'
			}
		],
		loading: false,
		noPagination: false,
		loadingPosition: 'bottom',
		useAction: true,
		useSelection: false,
		selectionKey: 'id',
		entityLabel: 'items',
		hidePaginationInfo: false
	}
)

const tableData = computed(() => {
	const raw = unref(props.data) ?? unref(props.datas) ?? []
	return Array.isArray(raw) ? raw : []
})

// 2. State to track sort column & direction
const currentSortKey = ref<string | null>(null)
const currentSortOrder = ref<TSortOrder>(null)

// 3. Function to handle click on sortable column headers
const handleSort = (column: ITableColumn) => {
	if (!column.sortable) return

	if (currentSortKey.value === column.key) {
		// Toggle sort order: asc -> desc -> null
		if (currentSortOrder.value === 'asc') {
			currentSortOrder.value = 'desc'
		} else if (currentSortOrder.value === 'desc') {
			currentSortOrder.value = null
			currentSortKey.value = null
		} else {
			currentSortOrder.value = 'asc'
		}
	} else {
		currentSortKey.value = column.key
		currentSortOrder.value = 'asc'
	}

	emits('update:sort', {
		key: currentSortKey.value,
		order: currentSortOrder.value
	})
}

const alignClassMap = (column: ITableColumn) => ({
	'text-left': column.align === 'left' || column.align === undefined,
	'text-center': column.align === 'center',
	'text-right': column.align === 'right'
})

const updatePage = (page: number, pageProps: any) => {
	const rows = pageProps?.rows ?? pageProps?.limit ?? 10
	const payload = {
		limit: rows,
		offset: (page - 1) * rows,
		page: page
	}

	emits('update:page', payload)
}

const colspanlength = computed(() => {
	let total = props.columns.length
	if (props.useAction) total += 1
	if (props.useSelection) total += 1
	return total
})

const isRowSelected = (row: any) => {
	if (!props.useSelection) return false
	const key = props.selectionKey
	const currentSelected = Array.isArray(selected.value) ? selected.value : []
	if (!key || row[key] === undefined) {
		const str = JSON.stringify(row)
		return currentSelected.some((item) => JSON.stringify(item) === str)
	}
	return currentSelected.some((item) => item[key] === row[key])
}

const isSelectedAll = computed(() => {
	if (!props.useSelection || tableData.value.length === 0) return false
	return tableData.value.every((row) => isRowSelected(row))
})

const isIndeterminate = computed(() => {
	if (!props.useSelection || tableData.value.length === 0) return false
	const selectedCount = tableData.value.filter((row) => isRowSelected(row)).length
	return selectedCount > 0 && selectedCount < tableData.value.length
})

const headerCheckboxRef = ref<any>(null)

const onToggleSelectAll = (checked?: boolean | 'indeterminate') => {
	const target = checked === true || (checked === undefined && !isSelectedAll.value)
	const key = props.selectionKey
	const currentSelected = Array.isArray(selected.value) ? [...selected.value] : []

	if (target) {
		tableData.value.forEach((row) => {
			if (!isRowSelected(row)) {
				currentSelected.push(row)
			}
		})
		selected.value = currentSelected
	} else {
		if (key) {
			const pageKeys = new Set(tableData.value.map((r) => r[key]))
			selected.value = currentSelected.filter((r) => !pageKeys.has(r[key]))
		} else {
			const pageStrings = new Set(tableData.value.map((r) => JSON.stringify(r)))
			selected.value = currentSelected.filter((r) => !pageStrings.has(JSON.stringify(r)))
		}
	}
}

const onToggleRow = (dt: any, checked: boolean | 'indeterminate') => {
	const key = props.selectionKey
	const currentSelected = Array.isArray(selected.value) ? [...selected.value] : []

	if (checked === true) {
		if (!isRowSelected(dt)) {
			currentSelected.push(dt)
			selected.value = currentSelected
		}
	} else {
		if (key && dt[key] !== undefined) {
			selected.value = currentSelected.filter((item) => item[key] !== dt[key])
		} else {
			const str = JSON.stringify(dt)
			selected.value = currentSelected.filter((item) => JSON.stringify(item) !== str)
		}
	}
}

// Pagination information calculations
const totalRecords = computed(() => {
	if (props.pagination?.totalRecords !== undefined) {
		return Number(props.pagination.totalRecords)
	}
	if (props.pagination?.total !== undefined) {
		return Number(props.pagination.total)
	}
	return tableData.value.length
})

const currentPage = computed(() => Number(props.pagination?.currentPage ?? 1))
const rowsPerPage = computed(() => Number(props.pagination?.rows ?? props.pagination?.limit ?? 10))

const fromRecord = computed(() => {
	if (totalRecords.value === 0) return 0
	return (currentPage.value - 1) * rowsPerPage.value + 1
})

const toRecord = computed(() => {
	if (totalRecords.value === 0) return 0
	return Math.min(currentPage.value * rowsPerPage.value, totalRecords.value)
})
</script>

<template>
	<section class="patrolli-table w-full">
		<div
			class="patrolli-table-container w-full overflow-x-auto rounded-lg border border-[#D8D9DB] dark:border-border bg-white dark:bg-card shadow-xs"
		>
			<table class="w-full caption-bottom text-sm border-collapse">
				<thead
					class="bg-[#F8FAFC] dark:bg-slate-900/60 border-b border-[#D8D9DB] dark:border-border"
				>
					<tr class="border-b border-[#D8D9DB] dark:border-border">
						<th v-if="useSelection" class="w-12 px-4 py-3 text-center align-middle">
							<div class="flex items-center justify-center">
								<UiCheckbox
									ref="headerCheckboxRef"
									:class="{ 'is-indeterminate': isIndeterminate }"
									:checked="isIndeterminate ? 'indeterminate' : isSelectedAll"
									@update:checked="onToggleSelectAll"
								/>
							</div>
						</th>

						<template v-for="column in columns" :key="column.key">
							<th
								class="px-4 py-3 text-left align-middle font-semibold text-xs tracking-wider text-[#616468] dark:text-muted-foreground uppercase select-none"
							>
								<slot
									:name="`head.${column.key}`"
									:data="column"
									:sort="() => handleSort(column)"
								>
									<div
										:class="[
											alignClassMap(column),
											{ 'cursor-pointer select-none': column.sortable }
										]"
										class="flex items-center justify-between gap-2"
										@click="handleSort(column)"
									>
										<div class="flex-1">
											{{ column.title ?? column.key }}
										</div>

										<!-- Dynamic Sort Icon Indicator -->
										<div
											v-if="column.sortable"
											class="inline-flex items-center"
										>
											<UiIcon
												v-if="
													currentSortKey === column.key &&
													currentSortOrder === 'asc'
												"
												name="sort-ascending"
												color="primary"
												size="16px"
											/>
											<UiIcon
												v-else-if="
													currentSortKey === column.key &&
													currentSortOrder === 'desc'
												"
												name="sort-descending"
												color="primary"
												size="16px"
											/>
											<UiIcon
												v-else
												name="sort"
												size="16px"
												class="opacity-40 hover:opacity-100 transition-opacity"
											/>
										</div>
									</div>
								</slot>
							</th>
						</template>

						<th
							v-if="useAction"
							class="px-4 py-3 text-right align-middle font-semibold text-xs tracking-wider text-[#616468] dark:text-muted-foreground uppercase"
						>
							<slot name="head.action">
								<div class="text-right">ACTIONS</div>
							</slot>
						</th>
					</tr>
				</thead>
				<tbody class="divide-y divide-[#EBEBEF] dark:divide-border/60">
					<tr v-if="loading && loadingPosition === 'top'">
						<td :colspan="colspanlength">
							<AppLoading container-height="32px" />
						</td>
					</tr>

					<template v-for="(dt, index) in tableData" :key="dt[selectionKey] || index">
						<tr
							class="hover:bg-slate-50/70 dark:hover:bg-muted/40 transition-colors"
							:class="{
								'is-selected bg-sky-50/40 dark:bg-sky-950/20': isRowSelected(dt)
							}"
						>
							<td
								v-if="useSelection"
								class="w-12 px-4 py-3.5 text-center align-middle"
							>
								<div class="flex items-center justify-center">
									<UiCheckbox
										:checked="isRowSelected(dt)"
										@update:checked="(val) => onToggleRow(dt, val)"
									/>
								</div>
							</td>

							<template v-for="column in props.columns" :key="column.key">
								<td class="px-4 py-3.5 align-middle text-sm text-foreground">
									<div :class="alignClassMap(column)">
										<slot :name="`col.${column.key}`" :data="dt">
											{{ dt[column.key] }}
										</slot>
									</div>
								</td>
							</template>

							<td v-if="useAction" class="px-4 py-3.5 align-middle text-right">
								<slot name="col.action" :data="dt">
									<div class="text-right">Action</div>
								</slot>
							</td>
						</tr>
					</template>

					<tr v-if="!loading && tableData.length === 0">
						<td :colspan="colspanlength" class="!py-12">
							<slot name="no-data">
								<div
									class="flex flex-col items-center justify-center text-slate-400 dark:text-slate-500 gap-1.5"
								>
									<div class="text-sm font-medium">Sorry! Data not found.</div>
								</div>
							</slot>
						</td>
					</tr>

					<tr v-if="loading && loadingPosition === 'bottom'">
						<td :colspan="colspanlength">
							<AppLoading container-height="32px" />
						</td>
					</tr>
				</tbody>
			</table>
		</div>

		<!-- Pagination Footer with dual-side layout -->
		<div
			v-if="!props.noPagination && props.pagination"
			class="patrolli-pagination-footer flex items-center justify-between mt-4"
		>
			<div
				v-if="!props.hidePaginationInfo"
				class="pagination-info text-xs text-muted-foreground"
			>
				<slot
					name="pagination-info"
					:from="fromRecord"
					:to="toRecord"
					:total="totalRecords"
				>
					Showing
					<span class="pagination-info-highlight font-semibold text-foreground"
						>{{ fromRecord }}-{{ toRecord }}</span
					>
					of
					<span class="pagination-info-highlight font-semibold text-foreground">{{
						totalRecords
					}}</span>
					{{ entityLabel }}
				</slot>
			</div>
			<div v-else></div>

			<UiPagination
				:total="totalRecords"
				:items-per-page="rowsPerPage"
				:page="currentPage"
				:sibling-count="1"
				show-edges
				@update:page="updatePage($event, props.pagination)"
			>
				<UiPaginationList v-slot="{ items }" class="flex items-center gap-1">
					<UiPaginationFirst />
					<UiPaginationPrev />
					<template v-for="(item, index) in items" :key="index">
						<UiPaginationListItem
							v-if="item.type === 'page'"
							:value="item.value"
							as-child
						>
							<UiButton
								class="w-8 h-8 p-0"
								:variant="item.value === currentPage ? 'default' : 'outline'"
								size="sm"
							>
								{{ item.value }}
							</UiButton>
						</UiPaginationListItem>
						<UiPaginationEllipsis v-else />
					</template>
					<UiPaginationNext />
					<UiPaginationLast />
				</UiPaginationList>
			</UiPagination>
		</div>
	</section>
</template>

<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed, ref, watch } from 'vue'

export interface TableItem {
	[key: string]: any
}

export interface TableHeader {
	key: string
	label?: string
	title?: string
}

const props = withDefaults(
	defineProps<{
		items: TableItem[]
		headers: TableHeader[]
		checkboxKey: string
		pagination?: any
		valueKey?: string
		multiple?: boolean
	}>(),
	{
		multiple: true
	}
)

const selectedItems = defineModel<TableItem[] | TableItem | null>({
	default: (props) => (props.multiple ? [] : null)
})

const selectedArray = computed<TableItem[]>(() => {
	if (!selectedItems.value) return []
	return Array.isArray(selectedItems.value) ? selectedItems.value : [selectedItems.value]
})

const isSelected = (item: TableItem) => {
	const valueKey = props.valueKey
	if (valueKey !== undefined) {
		return selectedArray.value.some((s) => s[valueKey] === item[valueKey])
	}
	return selectedArray.value.includes(item)
}

const toggleSelect = (val: boolean | 'indeterminate', item: TableItem) => {
	const boolVal = val === true
	const currentlySelected = isSelected(item)
	if (boolVal === currentlySelected) return

	if (!props.multiple) {
		selectedItems.value = boolVal ? item : null
	} else {
		const currentArr = Array.isArray(selectedItems.value) ? selectedItems.value : []
		if (boolVal) {
			selectedItems.value = [...currentArr, item]
		} else {
			const valueKey = props.valueKey
			if (valueKey !== undefined) {
				selectedItems.value = currentArr.filter((s) => s[valueKey] !== item[valueKey])
			} else {
				selectedItems.value = currentArr.filter((s) => s !== item)
			}
		}
	}
}

const selectAll = ref(false)
const toggleSelectAll = (val: boolean | 'indeterminate') => {
	if (!props.multiple) return
	const boolVal = val === true

	const valueKey = props.valueKey
	if (!boolVal) {
		if (valueKey !== undefined) {
			const currentItemsKeys = new Set(props.items.map((i) => i[valueKey]))
			selectedItems.value = selectedArray.value.filter(
				(s) => !currentItemsKeys.has(s[valueKey])
			)
		} else {
			selectedItems.value = selectedArray.value.filter((s) => !props.items.includes(s))
		}
	} else {
		if (valueKey !== undefined) {
			const selectedKeys = new Set(selectedArray.value.map((s) => s[valueKey]))
			const itemsToAdd = props.items.filter((item) => !selectedKeys.has(item[valueKey]))
			selectedItems.value = [...selectedArray.value, ...itemsToAdd]
		} else {
			const itemsToAdd = props.items.filter((item) => !selectedArray.value.includes(item))
			selectedItems.value = [...selectedArray.value, ...itemsToAdd]
		}
	}
}

watch(
	[() => selectedArray.value, () => props.items],
	() => {
		if (!props.multiple) return

		if (!props.items || props.items.length === 0) {
			selectAll.value = false
			return
		}

		const valueKey = props.valueKey
		if (valueKey !== undefined) {
			const allChecked = props.items.every((item) =>
				selectedArray.value.some((s) => s[valueKey] === item[valueKey])
			)
			selectAll.value = allChecked
		} else {
			selectAll.value = props.items.every((item) => selectedArray.value.includes(item))
		}
	},
	{ immediate: true }
)

const showdata = computed(() => props.items || [])

const isIndeterminate = computed(() => {
	if (!props.multiple || !props.items || props.items.length === 0) return false
	const selectedCount = props.items.filter((item) => isSelected(item)).length
	return selectedCount > 0 && selectedCount < props.items.length
})
</script>

<template>
	<section v-if="multiple" class="flex justify-between items-center mb-3 text-sm">
		<div class="flex items-center gap-2">
			<UiCheckbox
				:checked="isIndeterminate ? 'indeterminate' : selectAll"
				@update:checked="toggleSelectAll"
			/>
			<span class="text-xs font-medium text-muted-foreground select-none">
				Select All ({{ selectedArray.length }} Data)
			</span>
		</div>

		<span class="text-xs text-muted-foreground font-medium">
			Selected: <strong class="text-foreground">{{ selectedArray.length }}</strong> Data
		</span>
	</section>

	<div class="table-container mb-4 overflow-x-auto border border-border rounded-xl bg-card">
		<table class="w-full caption-bottom text-sm border-collapse">
			<thead>
				<tr class="border-b border-border bg-muted/40">
					<th
						v-for="header in headers"
						:key="header.key"
						class="h-10 px-4 text-left align-middle font-semibold text-xs uppercase tracking-wider text-muted-foreground"
					>
						{{ header.label ?? header.title }}
					</th>
				</tr>
			</thead>
			<tbody class="divide-y divide-border">
				<tr
					v-for="(item, i) in showdata"
					:key="i"
					:class="[
						'transition-colors hover:bg-muted/50',
						isSelected(item) ? 'bg-primary/5 dark:bg-primary/10' : ''
					]"
				>
					<td v-for="header in headers" :key="header.key" class="p-3 px-4 align-middle">
						<slot :name="header.key" :item="item" :value="item[header.key]">
							<div v-if="header.key === checkboxKey" class="flex items-center gap-2">
								<UiCheckbox
									:checked="isSelected(item)"
									@update:checked="(val) => toggleSelect(val, item)"
								/>
								<span class="text-sm font-medium">{{ item[header.key] }}</span>
							</div>
							<span v-else class="text-sm text-foreground">
								{{ item[header.key] }}
							</span>
						</slot>
					</td>
				</tr>

				<tr v-if="items.length === 0">
					<td
						:colspan="headers.length"
						class="p-8 text-center text-sm text-muted-foreground"
					>
						Tidak ada data tersedia.
					</td>
				</tr>
			</tbody>
		</table>
	</div>
</template>

<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { useValidation } from '@/composables/useValidation'
import AppInputError from '@/components/app/input/AppInputError.vue'
import { useDebounceFn, watchDebounced, watchOnce } from '@vueuse/core'
import { computed, onMounted, ref, watch } from 'vue'
import ScrollContainer from '@/components/app/ScrollContainer.vue'
import SelectTable, { type TableItem } from '@/components/app/SelectTable.vue'
import { cn } from '@/app/lib/utils'

const emits = defineEmits<{
	infinite: [payload: any]
	search: [search: string]
	close: []
	'update:error': [value: string]
}>()
const props = withDefaults(
	defineProps<{
		pagination: any
		label?: string
		showLabel?: boolean
		labelClass?: string
		headerTable?: any[]
		data: any[]
		required?: boolean
		valueKey: string
		checkboxKey?: string
		loading?: boolean
		multiple?: boolean
		error?: string | string[]
		rules?: string[]
		disabled?: boolean
		readonly?: boolean
	}>(),
	{
		headerTable: () => [
			{
				key: 'name',
				title: 'Name',
				sortable: true
			}
		],
		required: false,
		showLabel: true,
		checkboxKey: 'name',
		loading: true,
		multiple: true,
		error: '',
		disabled: false,
		readonly: false
	}
)

const showModal = ref(false)
const modelValue = defineModel<string[] | string | null | undefined>()
const selectedItem = ref<TableItem[] | TableItem | null | undefined>()
const search = ref<string>('')

const loadMore = (payload, selected) => {
	payload = {
		...payload,
		selected: selected
	}

	emits('infinite', payload)
}

const toggleShowModal = () => {
	if (props.disabled || props.readonly) return
	showModal.value = !showModal.value
}

const closeFormModal = () => {
	showModal.value = false
}

const onApply = () => {
	if (props.multiple) {
		const selectedArr = Array.isArray(selectedItem.value) ? selectedItem.value : []
		if (selectedArr.length > 0) {
			modelValue.value = selectedArr.map((v) => v[props.valueKey])
		} else {
			modelValue.value = undefined
		}
	} else {
		const selectedObj =
			selectedItem.value && !Array.isArray(selectedItem.value) ? selectedItem.value : null
		if (selectedObj) {
			modelValue.value = selectedObj[props.valueKey]
		} else {
			modelValue.value = undefined
		}
	}

	showModal.value = false
}

const syncSelectedItems = (currentData: any[]) => {
	if (!modelValue.value || currentData.length === 0) {
		selectedItem.value = props.multiple ? [] : null
		return
	}

	if (props.multiple) {
		const modelArr = Array.isArray(modelValue.value) ? modelValue.value : [modelValue.value]
		selectedItem.value = currentData.filter((item) =>
			item[props.valueKey]?.includeTexts
				? item[props.valueKey].includeTexts(modelArr)
				: modelArr.includes(item[props.valueKey])
		)
	} else {
		selectedItem.value =
			currentData.find((item) => item[props.valueKey] === modelValue.value) || null
	}
}

watch(
	() => showModal.value,
	(isOpen) => {
		if (isOpen) {
			syncSelectedItems(props.data)
			if (props.data.length == 0 && props.pagination.totalRecords != 0) {
				emits('search', '')
			}
		} else {
			search.value = ''
		}
	}
)

watch(
	() => props.data,
	(newData) => {
		syncSelectedItems(newData)
	},
	{ deep: true }
)

watch(
	() => modelValue.value,
	() => {
		syncSelectedItems(props.data)
	}
)

watchDebounced(
	() => search.value,
	async () => {
		if (showModal.value) {
			emits('search', search.value)
		}
	},
	{
		debounce: 500
	}
)

const debouncedFetch = useDebounceFn((refetch = false) => {
	const paylaod = {
		...props.pagination,
		page: 1,
		offset: 0,
		refetch: refetch
	}
	emits('infinite', paylaod)
}, 50)

let mountedTimestamp = 0
onMounted(() => {
	mountedTimestamp = performance.now()
	debouncedFetch()
})

watchOnce(
	() => modelValue.value,
	() => {
		const watchTimestamp = performance.now()
		if (mountedTimestamp > 0) {
			const duration = watchTimestamp - mountedTimestamp

			if (duration > 50 && duration < 1000) {
				debouncedFetch(true)
			} else {
				debouncedFetch()
			}
		} else {
			console.log('Terjadi sebelum mounted')
		}
	}
)

const rules = computed(() => {
	const r: string[] = [...(props.rules || [])]
	if (props.multiple) {
		if (props.required && !r.includes('array')) r.push('array')
	} else {
		if (props.required && !r.includes('required')) r.push('required')
	}
	return r
})

const { elRef, errorMessage, showMessages, handleAlertClose } = useValidation(
	props.label ?? '',
	modelValue,
	rules,
	{
		error: () => props.error,
		onErrorChange: (val) => emits('update:error', val),
		onClose: () => emits('close')
	}
)

const iconMap = {
	Host: 'server',
	Endpoint: 'api',
	Listener: 'api',
	Items: 'database',
	Processor: 'cog',
	Sink: 'server',
	'': 'cog'
}

const icon = computed(() => {
	const name = iconMap[props.label ?? '']
	return name
})
</script>

<template>
	<div v-bind="$attrs" ref="elRef" class="w-full space-y-1.5">
		<label
			v-if="showLabel && label"
			:class="cn('text-sm font-semibold text-foreground flex items-center gap-1', labelClass)"
		>
			<span>{{ label }}</span>
			<span v-if="required" class="text-destructive">*</span>
		</label>

		<div
			:class="
				cn(
					'flex h-9 w-full items-center justify-between rounded-md border border-input bg-transparent dark:bg-slate-900/40 px-3 py-1 text-sm shadow-sm transition-colors select-none focus:outline-none focus:ring-1 focus:ring-ring focus:border-input',
					props.disabled || props.readonly
						? 'opacity-60 cursor-not-allowed pointer-events-none bg-muted/50'
						: 'cursor-pointer hover:border-slate-400 dark:hover:border-slate-600',
					errorMessage && showMessages
						? 'border-destructive focus:ring-destructive ring-destructive'
						: ''
				)
			"
			:tabindex="props.disabled || props.readonly ? -1 : 0"
			@click="toggleShowModal"
			@keydown.enter.prevent="toggleShowModal"
			@keydown.space.prevent="toggleShowModal"
		>
			<div class="flex-1 flex justify-between items-center text-sm">
				<div class="flex-1 flex items-center gap-2">
					<span class="font-medium text-foreground">Select {{ label }}</span>

					<span class="text-xs text-muted-foreground">
						(
						<template v-if="props.multiple">
							{{ Array.isArray(modelValue) ? modelValue.length : 0 }}
						</template>
						<template v-else>
							{{ modelValue ? 1 : 0 }}
						</template>
						/ {{ pagination?.totalRecords ?? 0 }} Selected)
					</span>
				</div>

				<UiIcon name="chevron-down" size="16px" class="text-muted-foreground" />
			</div>
		</div>

		<AppInputError v-model="showMessages" :messages="errorMessage" @close="handleAlertClose" />
	</div>

	<UiDialog v-model:open="showModal">
		<UiDialogContent class="max-w-3xl max-h-[85vh] flex flex-col p-6">
			<UiDialogHeader>
				<div class="flex flex-row gap-3 items-center">
					<div
						class="flex items-center justify-center p-2 rounded-lg bg-accent text-accent-foreground"
					>
						<UiIcon :name="icon" size="20px" />
					</div>
					<UiDialogTitle class="text-lg font-semibold text-foreground">
						Select {{ props.label }}
					</UiDialogTitle>
				</div>
			</UiDialogHeader>

			<div class="relative w-full my-2">
				<UiIcon
					name="magnify"
					size="16px"
					class="absolute left-3 top-1/2 -translate-y-1/2 text-muted-foreground"
				/>
				<UiInput
					v-model="search"
					:placeholder="`Search by ${label?.toLocaleLowerCase()} name`"
					class="pl-9 h-9"
				/>
			</div>

			<ScrollContainer
				:is-fetching="loading"
				:max-height="'420px'"
				:pagination="pagination"
				class="flex-1 overflow-y-auto"
				@infinite="loadMore"
			>
				<section class="min-h-[300px]">
					<SelectTable
						v-model="selectedItem"
						:headers="headerTable"
						:items="data"
						:checkbox-key="checkboxKey"
						:pagination="pagination"
						:value-key="props.valueKey"
						:multiple="multiple"
					>
						<template #adm_status="{ item }">
							<div class="flex flex-row flex-wrap gap-2 items-center">
								<div
									class="w-2.5 h-2.5 rounded-full"
									:class="
										item.adm_status === 1 ? 'bg-emerald-500' : 'bg-destructive'
									"
								/>
								<span class="text-xs font-medium">
									{{ item.adm_status === 1 ? 'Active' : 'Inactive' }}
								</span>
							</div>
						</template>
					</SelectTable>
				</section>
			</ScrollContainer>

			<UiDialogFooter class="flex justify-end gap-2 pt-3 border-t border-border">
				<UiButton variant="outline" size="sm" @click="closeFormModal"> Cancel </UiButton>
				<UiButton variant="default" size="sm" @click="onApply"> Apply </UiButton>
			</UiDialogFooter>
		</UiDialogContent>
	</UiDialog>
</template>

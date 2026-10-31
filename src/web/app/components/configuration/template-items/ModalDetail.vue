<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { formatSchedule } from '@/app/helpers/schedule.helper'
import { useTemplateItemsStore } from '@/stores/template-items.store'
import type { IItem } from '@@/shared/types/item.type'
import { computed, ref, watch } from 'vue'

const props = defineProps<{
	showModal: boolean
	item: IItem | null
}>()

const emit = defineEmits(['dismiss'])

const templateItemsStore = useTemplateItemsStore()

const isModalShow = ref<boolean>(props.showModal)

const item = computed(() => props.item)

const getLabels = (item: IItem | null) => {
	if (!item) return {}
	const labels = item.labels || (item as any).resolved_labels
	if (!labels) return {}
	try {
		return typeof labels === 'string' ? JSON.parse(labels) : labels
	} catch (e) {
		return {}
	}
}

const getConfig = (item: IItem | null) => {
	if (!item) return {}
	let config = item.config || item.resolved_config
	if (!config && item.config_embed) {
		config = (item as any).config_embed.config || {}
	}
	if (!config && item.resolved_config_embed) {
		config = (item as any).resolved_config_embed.config || {}
	}
	try {
		return typeof config === 'string' ? JSON.parse(config) : config || {}
	} catch (e) {
		return {}
	}
}

const resolvedKeys = computed(() => {
	if (!item.value) return []
	return Object.keys(item.value).filter(
		(key) => key.includes('resolved') && (item.value as any)[key] !== null
	)
})

watch(
	() => props.showModal,
	(value) => {
		isModalShow.value = value
	}
)

watch(isModalShow, (value) => {
	if (!value) emit('dismiss')
})
</script>

<template>
	<UiDialog v-model:open="isModalShow">
		<UiDialogContent class="max-w-lg p-6 max-h-[85vh] flex flex-col gap-4">
			<UiDialogHeader>
				<div class="flex items-center gap-3">
					<div
						class="flex items-center justify-center p-2 rounded-lg bg-sky-100 text-sky-600 dark:bg-sky-950/50 dark:text-sky-400"
					>
						<UiIcon name="database" size="20px" />
					</div>
					<UiDialogTitle class="text-lg font-semibold text-foreground">
						Read One Item
					</UiDialogTitle>
				</div>
			</UiDialogHeader>

			<div class="flex flex-col gap-3 max-h-[calc(100vh-325px)] overflow-y-auto py-2">
				<div
					v-if="templateItemsStore.loading.form"
					key="loading"
					class="flex justify-center p-8"
				>
					<UiIcon name="refresh" class="animate-spin text-primary" size="24px" />
				</div>
				<div v-else-if="item" key="content" class="flex flex-col gap-3 text-sm">
					<div class="flex flex-row items-center">
						<div class="text-sm font-medium text-muted-foreground w-[140px]">
							Item ID
						</div>
						<div class="text-sm font-normal text-foreground flex-1">
							: {{ item?.item_id || '-' }}
						</div>
					</div>

					<div class="flex flex-row items-center" v-for="key in resolvedKeys" :key="key">
						<div class="text-sm font-medium text-muted-foreground w-[140px] capitalize">
							{{
								key
									.replace(/_/g, ' ')
									.replace('adm status', 'status')
									.replace('timeout ms', 'timeout')
							}}
						</div>
						<div
							class="text-sm font-normal flex flex-row gap-2 items-start flex-wrap flex-1"
						>
							:
							<span v-if="key === 'resolved_schedule'">{{
								formatSchedule((item as any)[key])
							}}</span>
							<span v-else-if="key === 'resolved_adm_status'">{{
								Number((item as any)[key]) === 1 ? 'Active' : 'Inactive'
							}}</span>
							<span v-else-if="key === 'resolved_outputs'">{{
								(item as any)[key]?.length || 0
							}}</span>
							<span v-else-if="key === 'resolved_labels'">{{
								JSON.stringify(getLabels(item))
							}}</span>
							<span
								v-else-if="
									key === 'resolved_config' || key === 'resolved_config_embed'
								"
								>{{ JSON.stringify(getConfig(item)) }}</span
							>
							<span v-else>{{ (item as any)[key] }}</span>
						</div>
					</div>

					<div class="flex flex-row items-center">
						<div class="text-sm font-medium text-muted-foreground w-[140px]">
							Template Name
						</div>
						<div class="text-sm font-normal text-foreground flex-1">
							: {{ item?.template_name || item?.template_id || '-' }}
						</div>
					</div>

					<div class="flex flex-row items-center">
						<div class="text-sm font-medium text-muted-foreground w-[140px]">Type</div>
						<div class="text-sm font-normal text-foreground flex-1">
							:
							<span v-if="item?.type">{{
								String(item.type)
									.split('_')
									.map((word) => word.charAt(0).toUpperCase() + word.slice(1))
									.join(' ')
							}}</span>
							<span v-else class="text-sm">-</span>
						</div>
					</div>

					<div
						class="flex flex-col gap-1.5 w-full mt-1"
						v-if="Object.keys(getConfig(item)).length > 0"
					>
						<div class="text-sm font-medium text-muted-foreground">Type Config:</div>
						<div
							class="text-xs bg-muted p-3 rounded-lg max-h-40 overflow-auto whitespace-pre-wrap font-mono border"
						>
							{{ JSON.stringify(getConfig(item), null, 2) }}
						</div>
					</div>

					<div
						class="flex"
						:class="
							Object.keys(getLabels(item)).length === 0
								? 'flex-row items-center'
								: 'flex-col gap-2'
						"
					>
						<div class="text-sm font-medium text-muted-foreground w-[140px] shrink-0">
							Labels
						</div>
						<div class="flex gap-1.5 flex-wrap flex-1">
							<template v-if="Object.keys(getLabels(item)).length > 0">
								<UiBadge
									v-for="(value, key) in getLabels(item)"
									:key="`detail-labels-${key}-${value}`"
									variant="secondary"
									class="text-xs"
								>
									{{ key }} : {{ value }}
								</UiBadge>
							</template>
							<div v-else class="text-sm font-normal text-muted-foreground">: -</div>
						</div>
					</div>
				</div>
			</div>

			<UiDialogFooter class="flex items-center justify-end pt-3 border-t">
				<UiButton variant="outline" size="sm" @click="emit('dismiss')"> Close </UiButton>
			</UiDialogFooter>
		</UiDialogContent>
	</UiDialog>
</template>

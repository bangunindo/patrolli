<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { formatUtcDate } from '@/app/helpers/date.helper'
import type { ISink } from '@@/shared/types/sink.type'
import { computed, ref, watch } from 'vue'

const props = defineProps<{
	showModal: boolean
	sink: ISink | null
}>()

const emit = defineEmits(['dismiss'])

const isModalShow = ref<boolean>(props.showModal)

watch(
	() => props.showModal,
	(value) => {
		isModalShow.value = value
	}
)

watch(isModalShow, (value) => {
	if (!value) emit('dismiss')
})

const parsedLabels = computed(() => {
	if (!props.sink?.labels) return {}
	try {
		return typeof props.sink.labels === 'string'
			? JSON.parse(props.sink.labels)
			: props.sink.labels
	} catch (e) {
		return {}
	}
})

const hasLabels = computed(() => {
	return Object.keys(parsedLabels.value).length > 0
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
						<UiIcon name="server" size="20px" />
					</div>
					<UiDialogTitle class="text-lg font-semibold text-foreground">
						Read One Sink
					</UiDialogTitle>
				</div>
			</UiDialogHeader>

			<div class="flex flex-col gap-3 max-h-[calc(100vh-325px)] overflow-y-auto py-2">
				<div v-if="sink" class="flex flex-col gap-3 text-sm">
					<div class="flex flex-row items-center">
						<div class="text-sm font-medium text-muted-foreground w-[120px]">
							Sink ID
						</div>
						<div class="text-sm font-normal text-foreground flex-1">
							: {{ sink.sink_id }}
						</div>
					</div>

					<div class="flex flex-row items-center">
						<div class="text-sm font-medium text-muted-foreground w-[120px]">
							Sink Name
						</div>
						<div class="text-sm font-normal text-foreground flex-1">
							: {{ sink.name }}
						</div>
					</div>

					<div class="flex flex-row items-center">
						<div class="text-sm font-medium text-muted-foreground w-[120px]">
							Status
						</div>
						<div class="text-sm font-normal flex items-center gap-2 flex-1">
							:
							<div
								class="w-2.5 h-2.5 rounded-full"
								:class="
									Number(sink.adm_status) === 1
										? 'bg-emerald-500'
										: 'bg-destructive'
								"
							></div>
							<span>{{ Number(sink.adm_status) === 1 ? 'Active' : 'Inactive' }}</span>
						</div>
					</div>

					<div class="flex flex-row items-center">
						<div class="text-sm font-medium text-muted-foreground w-[120px]">
							Worker Count
						</div>
						<div class="text-sm font-normal text-foreground flex-1">
							: {{ sink.worker_count ?? 0 }}
						</div>
					</div>

					<div class="flex flex-row items-center">
						<div class="text-sm font-medium text-muted-foreground w-[120px]">
							Input Count
						</div>
						<div class="text-sm font-normal text-foreground flex-1">
							: {{ (sink as any).input_count ?? 0 }}
						</div>
					</div>

					<div class="flex flex-row items-center">
						<div class="text-sm font-medium text-muted-foreground w-[120px]">
							Last Updated
						</div>
						<div class="text-sm font-normal text-muted-foreground flex-1">
							: {{ formatUtcDate(sink.last_updated) }}
						</div>
					</div>

					<div class="flex flex-row items-center">
						<div class="text-sm font-medium text-muted-foreground w-[120px]">Type</div>
						<div class="text-sm font-normal text-foreground flex-1">
							: {{ sink.config?.type || '-' }}
						</div>
					</div>

					<div class="flex flex-col gap-1.5">
						<div class="text-sm font-medium text-muted-foreground">Type Config:</div>
						<div
							class="text-xs bg-muted p-3 rounded-lg max-h-40 overflow-auto whitespace-pre-wrap font-mono border"
						>
							{{
								(sink.config as any)?.config
									? JSON.stringify((sink.config as any).config, null, 2)
									: '-'
							}}
						</div>
					</div>

					<div class="flex flex-col gap-1.5">
						<div class="text-sm font-medium text-muted-foreground">Batch:</div>
						<div
							class="text-xs bg-muted p-3 rounded-lg max-h-40 overflow-auto whitespace-pre-wrap font-mono border"
						>
							{{ sink.batch ? JSON.stringify(sink.batch, null, 2) : '-' }}
						</div>
					</div>

					<div class="flex flex-col gap-1.5">
						<div class="text-sm font-medium text-muted-foreground">Channel:</div>
						<div
							class="text-xs bg-muted p-3 rounded-lg max-h-40 overflow-auto whitespace-pre-wrap font-mono border"
						>
							{{ sink.channel ? JSON.stringify(sink.channel, null, 2) : '-' }}
						</div>
					</div>

					<div
						class="flex"
						:class="!hasLabels ? 'flex-row items-center' : 'flex-col gap-2'"
					>
						<div class="text-sm font-medium text-muted-foreground w-[120px] shrink-0">
							Labels
						</div>
						<div class="flex gap-1.5 flex-wrap flex-1">
							<template v-if="hasLabels">
								<UiBadge
									v-for="(value, key) in parsedLabels"
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

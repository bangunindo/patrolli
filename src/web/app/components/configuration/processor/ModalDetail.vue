<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { ref, watch } from 'vue'
import type { IProcessor } from '@@/shared/types/processor.type'
import { formatUtcDate } from '@/app/helpers/date.helper'

const props = defineProps<{
	showModal: boolean
	processor: IProcessor | null
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
</script>

<template>
	<UiDialog v-model:open="isModalShow">
		<UiDialogContent class="max-w-lg p-6 max-h-[85vh] flex flex-col gap-4">
			<UiDialogHeader>
				<div class="flex items-center gap-3">
					<div
						class="flex items-center justify-center p-2 rounded-lg bg-sky-100 text-sky-600 dark:bg-sky-950/50 dark:text-sky-400"
					>
						<UiIcon name="cog" size="20px" />
					</div>
					<UiDialogTitle class="text-lg font-semibold text-foreground">
						Read One Processor
					</UiDialogTitle>
				</div>
			</UiDialogHeader>

			<div class="flex flex-col gap-3 max-h-[calc(100vh-325px)] overflow-y-auto py-2">
				<div v-if="processor" class="flex flex-col gap-3 text-sm">
					<div class="flex flex-row items-center">
						<div class="text-sm font-medium text-muted-foreground w-[120px]">
							Processor ID
						</div>
						<div class="text-sm font-normal text-foreground flex-1">
							: {{ processor.processor_id ?? (processor as any).pipeline_id }}
						</div>
					</div>

					<div class="flex flex-row items-center">
						<div class="text-sm font-medium text-muted-foreground w-[120px]">
							Processor Name
						</div>
						<div class="text-sm font-normal text-foreground flex-1">
							: {{ processor.name }}
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
									Number(processor.adm_status) === 1
										? 'bg-emerald-500'
										: 'bg-destructive'
								"
							></div>
							<span>{{
								Number(processor.adm_status) === 1 ? 'Active' : 'Inactive'
							}}</span>
						</div>
					</div>

					<div class="flex flex-row items-center">
						<div class="text-sm font-medium text-muted-foreground w-[120px]">
							Last Updated
						</div>
						<div class="text-sm font-normal text-muted-foreground flex-1">
							: {{ formatUtcDate(processor.last_updated) }}
						</div>
					</div>

					<div class="flex flex-row items-center">
						<div class="text-sm font-medium text-muted-foreground w-[120px]">
							Module
						</div>
						<div class="text-sm font-normal text-foreground capitalize flex-1">
							:
							{{
								processor.config?.module
									? processor.config.module.replace(/_/g, ' ')
									: '-'
							}}
						</div>
					</div>

					<div class="flex flex-col gap-1.5">
						<div class="text-sm font-medium text-muted-foreground">Module Config:</div>
						<div
							class="text-xs bg-muted p-3 rounded-lg max-h-40 overflow-auto whitespace-pre-wrap font-mono border"
						>
							{{
								(processor.config as any)?.config
									? JSON.stringify((processor.config as any).config, null, 2)
									: '-'
							}}
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

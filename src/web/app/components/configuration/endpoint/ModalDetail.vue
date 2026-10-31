<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { formatUtcDate } from '@/app/helpers/date.helper'
import type { IEndpoint } from '@@/shared/types/endpoint.type'
import { ref, watch } from 'vue'

const props = defineProps<{
	showModal: boolean
	endpoint: IEndpoint | null
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
						<UiIcon name="earth" size="20px" />
					</div>
					<UiDialogTitle class="text-lg font-semibold text-foreground">
						Read One Endpoint
					</UiDialogTitle>
				</div>
			</UiDialogHeader>

			<div class="flex flex-col gap-3 max-h-[calc(100vh-325px)] overflow-y-auto py-2">
				<div v-if="endpoint" class="flex flex-col gap-3 text-sm">
					<div class="flex flex-row items-center">
						<div class="text-sm font-medium text-muted-foreground w-[120px]">
							Endpoint ID
						</div>
						<div class="text-sm font-normal text-foreground flex-1">
							: {{ endpoint.endpoint_id }}
						</div>
					</div>

					<div class="flex flex-row items-center">
						<div class="text-sm font-medium text-muted-foreground w-[120px]">
							Endpoint Name
						</div>
						<div class="text-sm font-normal text-foreground flex-1">
							: {{ endpoint.name }}
						</div>
					</div>

					<div class="flex flex-row items-center">
						<div class="text-sm font-medium text-muted-foreground w-[120px]">Host</div>
						<div class="text-sm font-normal text-foreground flex-1">
							: {{ (endpoint as any).host_name || endpoint.host_id || '-' }}
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
									Number(endpoint.adm_status) === 1
										? 'bg-emerald-500'
										: 'bg-destructive'
								"
							></div>
							<span>{{
								Number(endpoint.adm_status) === 1 ? 'Active' : 'Inactive'
							}}</span>
						</div>
					</div>

					<div class="flex flex-row items-center">
						<div class="text-sm font-medium text-muted-foreground w-[120px]">
							Last Updated
						</div>
						<div class="text-sm font-normal text-muted-foreground flex-1">
							: {{ formatUtcDate(endpoint.last_updated) }}
						</div>
					</div>

					<div class="flex flex-row items-center">
						<div class="text-sm font-medium text-muted-foreground w-[120px]">Type</div>
						<div class="text-sm font-normal text-foreground flex-1">
							: {{ endpoint.config?.type || '-' }}
						</div>
					</div>

					<div class="flex flex-col gap-1.5">
						<div class="text-sm font-medium text-muted-foreground">Type Config:</div>
						<div
							class="text-xs bg-muted p-3 rounded-lg max-h-40 overflow-auto whitespace-pre-wrap font-mono border"
						>
							{{
								endpoint.config?.config
									? JSON.stringify(endpoint.config.config, null, 2)
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

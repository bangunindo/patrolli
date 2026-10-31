<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { formatUtcDate } from '@/app/helpers/date.helper'
import type { IHost } from '@@/shared/types/host.type'
import { computed, ref, watch } from 'vue'

const props = defineProps<{
	showModal: boolean
	host: IHost | null
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
	if (!props.host?.labels) return {}
	try {
		return typeof props.host.labels === 'string'
			? JSON.parse(props.host.labels)
			: props.host.labels
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
						Read One Host
					</UiDialogTitle>
				</div>
			</UiDialogHeader>

			<div class="flex flex-col gap-3 max-h-[calc(100vh-325px)] overflow-y-auto py-2">
				<div v-if="host" class="flex flex-col gap-3 text-sm">
					<div class="flex flex-row items-center">
						<div class="text-sm font-medium text-muted-foreground w-[120px]">
							Host ID
						</div>
						<div class="text-sm font-normal text-foreground flex-1">
							: {{ host.host_id }}
						</div>
					</div>

					<div class="flex flex-row items-center">
						<div class="text-sm font-medium text-muted-foreground w-[120px]">
							Host Name
						</div>
						<div class="text-sm font-normal text-foreground flex-1">
							: {{ host.name }}
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
									Number(host.adm_status) === 1
										? 'bg-emerald-500'
										: 'bg-destructive'
								"
							></div>
							<span>{{ Number(host.adm_status) === 1 ? 'Active' : 'Inactive' }}</span>
						</div>
					</div>

					<div class="flex flex-row items-center">
						<div class="text-sm font-medium text-muted-foreground w-[120px]">
							Last Updated
						</div>
						<div class="text-sm font-normal text-muted-foreground flex-1">
							: {{ formatUtcDate(host.last_updated) }}
						</div>
					</div>

					<div
						class="flex"
						:class="!hasLabels ? 'flex-row items-center' : 'flex-col gap-2'"
					>
						<div class="text-sm font-medium text-muted-foreground w-[120px]">
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

					<div
						class="flex"
						:class="
							!host.templates?.length ? 'flex-row items-center' : 'flex-col gap-2'
						"
					>
						<div class="text-sm font-medium text-muted-foreground w-[120px] shrink-0">
							Templates
						</div>
						<div
							class="flex flex-row flex-wrap gap-1.5 flex-1"
							v-if="host.templates?.length"
						>
							<UiBadge
								v-for="template in host.templates"
								variant="outline"
								:key="template.template_id"
								class="text-xs"
							>
								{{ template.name || template.template_id }}
							</UiBadge>
						</div>
						<div v-else class="text-sm font-normal text-muted-foreground">: -</div>
					</div>

					<div class="flex flex-row items-center">
						<div class="text-sm font-medium text-muted-foreground w-[120px]">
							Item Count
						</div>
						<div class="text-sm font-normal text-foreground flex-1">
							: {{ host.item_count || 0 }}
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

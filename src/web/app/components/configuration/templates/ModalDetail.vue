<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import type { ITemplate } from '@@/shared/types/template.type'
import { computed, ref, watch } from 'vue'

const props = defineProps<{
	showModal: boolean
	template: ITemplate | null
}>()

const emit = defineEmits(['dismiss', 'open-template'])

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
	if (!props.template?.labels) return {}
	try {
		return typeof props.template.labels === 'string'
			? JSON.parse(props.template.labels)
			: props.template.labels
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
						<UiIcon name="file-document" size="20px" />
					</div>
					<UiDialogTitle class="text-lg font-semibold text-foreground">
						Read One Template
					</UiDialogTitle>
				</div>
			</UiDialogHeader>

			<div class="flex flex-col gap-3 max-h-[calc(100vh-325px)] overflow-y-auto py-2">
				<div v-if="template" class="flex flex-col gap-3 text-sm">
					<div class="flex flex-row items-center">
						<div class="text-sm font-medium text-muted-foreground w-[150px]">
							Template Name
						</div>
						<div class="text-sm font-normal text-foreground flex-1">
							: {{ template.name || '-' }}
						</div>
					</div>

					<div
						class="flex"
						:class="!hasLabels ? 'flex-row items-center' : 'flex-col gap-2'"
					>
						<div class="text-sm font-medium text-muted-foreground w-[150px] shrink-0">
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
							!template.links_to_templates?.length
								? 'flex-row items-center'
								: 'flex-col gap-2'
						"
					>
						<div class="text-sm font-medium text-muted-foreground w-[150px] shrink-0">
							Links to Template
						</div>
						<div class="flex gap-1.5 flex-wrap flex-1">
							<template v-if="template.links_to_templates?.length">
								<a
									v-for="link in template.links_to_templates"
									class="cursor-pointer"
									:key="link.template_id"
									@click.prevent="emit('open-template', link.template_id)"
								>
									<UiBadge variant="outline" class="text-xs hover:bg-accent">
										{{ link.name }}
									</UiBadge>
								</a>
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

<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { createReusableTemplate } from '@vueuse/core'
import { Alert as UiAlert, AlertDescription as UiAlertDescription } from '@/components/ui/alert'
import { Badge as UiBadge } from '@/components/ui/badge'
import { Button as UiButton } from '@/components/ui/button'
import {
	Card as UiCard,
	CardContent as UiCardContent,
	CardFooter as UiCardFooter,
	CardHeader as UiCardHeader
} from '@/components/ui/card'
import { Checkbox as UiCheckbox } from '@/components/ui/checkbox'
import {
	Dialog as UiDialog,
	DialogContent as UiDialogContent,
	DialogDescription as UiDialogDescription,
	DialogFooter as UiDialogFooter,
	DialogHeader as UiDialogHeader,
	DialogTitle as UiDialogTitle
} from '@/components/ui/dialog'
import UiIcon from '@/components/ui/icon/Icon.vue'
import { Input as UiInput } from '@/components/ui/input'
import { ScrollArea as UiScrollArea } from '@/components/ui/scroll-area'
import { usePipelineStore } from '@/stores/pipeline.store'
import type { ISinkSelectorItem } from '@@/shared/types/pipeline.type'

const props = withDefaults(
	defineProps<{
		modelValue?: boolean
		inline?: boolean
		suspended?: boolean
		sinks?: ISinkSelectorItem[]
		selectedIds?: string[]
		loading?: boolean
		saving?: boolean
		errorMessage?: string | null
	}>(),
	{
		modelValue: true,
		inline: false,
		suspended: false,
		sinks: undefined,
		selectedIds: undefined,
		loading: false,
		saving: false,
		errorMessage: null
	}
)

const emit = defineEmits<{
	(e: 'update:modelValue', val: boolean): void
	(e: 'close'): void
	(e: 'apply', ids: string[]): void
	(e: 'clearError'): void
}>()

const pipelineStore = usePipelineStore()

const search = ref('')
const draftSelected = ref<string[]>([])

const isControlled = computed(() => props.selectedIds !== undefined)
const sinkList = computed(() => props.sinks ?? pipelineStore.availableSinks)
const draftSelectedSet = computed(() => new Set(draftSelected.value))
const sourceIds = computed(() => props.selectedIds ?? pipelineStore.form.assignedSinks)
const sourceSignature = computed(() => sourceIds.value.join('\0'))

watch(
	[() => props.modelValue, () => props.inline, sourceSignature] as const,
	([isOpen, isInline], prev) => {
		if (isOpen || isInline) {
			draftSelected.value = [...sourceIds.value]
			const wasActive = prev ? Boolean(prev[0] || prev[1]) : false
			if (!wasActive) {
				search.value = ''
			}
		}
	},
	{ immediate: true }
)

const filteredSinks = computed(() => {
	const q = search.value.toLowerCase().trim()
	if (q === '') return sinkList.value
	return sinkList.value.filter((sink: ISinkSelectorItem) => {
		return (
			sink.name.toLowerCase().includes(q) ||
			Boolean(sink.type && sink.type.toLowerCase().includes(q))
		)
	})
})

function setSinkChecked(sinkId: string, checked = !isSinkChecked(sinkId)) {
	if (props.saving) return
	emit('clearError')
	const idx = draftSelected.value.indexOf(sinkId)
	if (checked && idx === -1) {
		draftSelected.value.push(sinkId)
	} else if (!checked && idx > -1) {
		draftSelected.value.splice(idx, 1)
	}
}

function isSinkChecked(sinkId: string): boolean {
	return draftSelectedSet.value.has(sinkId)
}

function selectAllFiltered() {
	if (props.saving || props.loading) return
	emit('clearError')
	const currentFilteredIds = filteredSinks.value.map((s) => s.id)
	if (currentFilteredIds.length === 0) return
	const selectedSet = draftSelectedSet.value
	const allInDraft = currentFilteredIds.every((id) => selectedSet.has(id))

	if (allInDraft) {
		const filteredSet = new Set(currentFilteredIds)
		draftSelected.value = draftSelected.value.filter((id) => !filteredSet.has(id))
	} else {
		for (const id of currentFilteredIds) {
			if (!selectedSet.has(id)) draftSelected.value.push(id)
		}
	}
}

function clearAll() {
	if (props.saving) return
	emit('clearError')
	draftSelected.value = []
}

function applySelection() {
	if (isControlled.value) {
		emit('apply', [...draftSelected.value])
		return
	}

	pipelineStore.form.assignedSinks = [...draftSelected.value]
	close()
}

const [DefineSelectorBody, SelectorBody] = createReusableTemplate()
const [DefineSelectorActions, SelectorActions] = createReusableTemplate()

function close() {
	if (props.saving) return
	emit('update:modelValue', false)
	emit('close')
}

function handleDialogOpenChange(open: boolean) {
	if (!open) {
		close()
	}
}
</script>

<template>
	<DefineSelectorBody>
		<div class="p-3 border-b border-border flex flex-col gap-2 bg-muted/40">
			<div class="flex items-center gap-2">
				<div class="relative flex-1">
					<UiIcon
						name="magnify"
						size="14px"
						class="absolute left-2.5 top-1/2 -translate-y-1/2 text-muted-foreground pointer-events-none"
					/>
					<UiInput
						v-model="search"
						type="text"
						placeholder="Search sink by name or type..."
						class="h-8 pl-8 pr-3 text-xs"
						:disabled="saving"
					/>
				</div>

				<UiButton
					variant="outline"
					size="xs"
					class="shrink-0"
					:disabled="saving || loading || filteredSinks.length === 0"
					@click="selectAllFiltered"
				>
					Select All
				</UiButton>
			</div>

			<UiAlert v-if="errorMessage" variant="destructive" class="py-2 px-3">
				<UiAlertDescription class="text-[11px] font-medium">
					{{ errorMessage }}
				</UiAlertDescription>
			</UiAlert>
		</div>

		<UiScrollArea class="h-[220px]">
			<div class="divide-y divide-border p-1">
				<div
					v-if="loading && sinkList.length === 0"
					class="py-8 text-center text-xs text-muted-foreground"
				>
					Loading sinks...
				</div>

				<div
					v-for="sink in filteredSinks"
					v-else
					:key="sink.id"
					class="flex items-center justify-between px-3 py-2 rounded-md transition-colors cursor-pointer"
					:class="
						isSinkChecked(sink.id)
							? 'bg-sky-50/80 dark:bg-sky-950/40'
							: 'hover:bg-accent/60'
					"
					@click="setSinkChecked(sink.id)"
				>
					<div class="flex items-center gap-2.5 min-w-0">
						<UiCheckbox
							:model-value="isSinkChecked(sink.id)"
							:disabled="saving"
							aria-label="Select sink"
							@click.stop
							@update:model-value="(val) => setSinkChecked(sink.id, Boolean(val))"
						/>
						<span
							class="text-xs truncate"
							:class="
								isSinkChecked(sink.id)
									? 'font-semibold text-foreground'
									: 'font-normal text-foreground/80'
							"
						>
							{{ sink.name }}
						</span>
					</div>

					<UiBadge
						:variant="isSinkChecked(sink.id) ? 'outline' : 'secondary'"
						class="px-2 py-0.5 text-[10px] uppercase font-mono font-normal shrink-0"
						:class="
							isSinkChecked(sink.id)
								? 'bg-sky-100 dark:bg-sky-950 text-sky-700 dark:text-sky-300 border-sky-200 dark:border-sky-800'
								: ''
						"
					>
						{{ sink.type }}
					</UiBadge>
				</div>

				<div
					v-if="!loading && filteredSinks.length === 0"
					class="py-8 text-center text-xs text-muted-foreground"
				>
					No sinks found matching filter
				</div>
			</div>
		</UiScrollArea>
	</DefineSelectorBody>

	<DefineSelectorActions>
		<UiButton variant="outline" size="xs" :disabled="saving" @click="clearAll">
			Clear All
		</UiButton>
		<UiButton size="xs" :disabled="saving || loading" @click="applySelection">
			{{ saving ? 'Saving...' : `Apply Selection (${draftSelected.length})` }}
		</UiButton>
	</DefineSelectorActions>

	<UiCard v-if="inline" class="w-full overflow-hidden shadow-xl">
		<UiCardHeader
			class="flex flex-row items-center justify-between space-y-0 px-4 py-3 border-b border-border"
		>
			<div class="flex items-center gap-2">
				<span class="text-xs font-semibold text-foreground"> Select Sinks </span>
				<UiBadge
					variant="secondary"
					class="px-2 py-0.5 text-[10px] font-medium bg-sky-100 dark:bg-sky-950 text-sky-700 dark:text-sky-300"
				>
					{{ draftSelected.length }} of {{ sinkList.length }} Selected
				</UiBadge>
			</div>
			<UiButton
				variant="ghost"
				size="icon"
				class="h-6 w-6 text-muted-foreground hover:text-foreground"
				:disabled="saving"
				@click="close"
			>
				<UiIcon name="close" size="14px" />
			</UiButton>
		</UiCardHeader>

		<UiCardContent class="p-0">
			<SelectorBody />
		</UiCardContent>

		<UiCardFooter
			class="flex items-center justify-between px-4 py-3 border-t border-border bg-muted/40"
		>
			<SelectorActions />
		</UiCardFooter>
	</UiCard>

	<UiDialog v-else :open="modelValue && !suspended" @update:open="handleDialogOpenChange">
		<UiDialogContent
			class="sm:max-w-[490px] p-0 gap-0 overflow-hidden"
			@escape-key-down="(e) => saving && e.preventDefault()"
			@pointer-down-outside="(e) => saving && e.preventDefault()"
			@interact-outside="(e) => saving && e.preventDefault()"
		>
			<UiDialogHeader
				class="px-4 py-3 border-b border-border flex-row items-center gap-2 space-y-0 pr-10"
			>
				<UiDialogTitle class="text-xs font-semibold text-foreground">
					Select Sinks
				</UiDialogTitle>
				<UiBadge
					variant="secondary"
					class="px-2 py-0.5 text-[10px] font-medium bg-sky-100 dark:bg-sky-950 text-sky-700 dark:text-sky-300"
				>
					{{ draftSelected.length }} of {{ sinkList.length }} Selected
				</UiBadge>
				<UiDialogDescription class="sr-only">
					Select sinks to attach to this pipeline
				</UiDialogDescription>
			</UiDialogHeader>

			<SelectorBody />

			<UiDialogFooter
				class="flex-row items-center justify-between sm:justify-between px-4 py-3 border-t border-border bg-muted/40 gap-2"
			>
				<SelectorActions />
			</UiDialogFooter>
		</UiDialogContent>
	</UiDialog>
</template>

<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
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
import { useResourceSelector, type ResourceSelectorItem } from '@/composables/useResourceSelector'

const props = withDefaults(
	defineProps<{
		modelValue?: boolean
		inline?: boolean
		suspended?: boolean
		items: ResourceSelectorItem[]
		totalCount?: number
		sourceIds: string[]
		loading?: boolean
		loadingMore?: boolean
		saving?: boolean
		minSelection?: number
		errorMessage?: string | null
		title: string
		description: string
		searchPlaceholder: string
		singularLabel: string
		pluralLabel: string
		badgeUppercase?: boolean
		onSearch?: (query: string) => Promise<void>
		onLoadMore?: () => Promise<void>
		onResolveAllIds?: (searchQuery?: string) => Promise<string[]>
	}>(),
	{
		modelValue: true,
		inline: false,
		suspended: false,
		totalCount: undefined,
		loading: false,
		loadingMore: false,
		saving: false,
		minSelection: 0,
		errorMessage: null,
		badgeUppercase: false,
		onSearch: undefined,
		onLoadMore: undefined,
		onResolveAllIds: undefined
	}
)

const emit = defineEmits<{
	(e: 'update:modelValue', val: boolean): void
	(e: 'close'): void
	(e: 'commit', ids: string[]): void
	(e: 'clearError'): void
}>()

const {
	search,
	isBusy,
	isSearchPending,
	resolvingAll,
	displayError,
	effectiveTotal,
	selectedCount,
	badgeTotal,
	hasMore,
	filteredItems,
	renderedItems,
	containerProps,
	wrapperProps,
	handleListScroll,
	isChecked,
	setChecked,
	selectAllFiltered,
	clearAll,
	applySelection
} = useResourceSelector({
	isOpen: () => props.modelValue,
	isInline: () => props.inline,
	items: () => props.items,
	totalCount: () => props.totalCount ?? props.items.length,
	sourceIds: () => props.sourceIds,
	loading: () => props.loading,
	loadingMore: () => props.loadingMore,
	saving: () => props.saving,
	minSelection: () => props.minSelection,
	errorMessage: () => props.errorMessage,
	singularLabel: props.singularLabel,
	pluralLabel: props.pluralLabel,
	searchHandler: () => props.onSearch,
	loadMoreHandler: () => props.onLoadMore,
	resolveAllHandler: () => props.onResolveAllIds,
	onClearError: () => emit('clearError'),
	onCommit: (ids) => emit('commit', ids)
})

const [DefineSelectorBody, SelectorBody] = createReusableTemplate()
const [DefineSelectorActions, SelectorActions] = createReusableTemplate()

function close() {
	if (isBusy.value) return
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
						:placeholder="searchPlaceholder"
						class="h-8 pl-8 pr-3 text-xs"
						:disabled="isBusy"
					/>
				</div>

				<UiButton
					variant="outline"
					size="xs"
					class="shrink-0"
					:disabled="isBusy || loading || isSearchPending || filteredItems.length === 0"
					@click="selectAllFiltered"
				>
					Select All
				</UiButton>
			</div>

			<UiAlert v-if="displayError" variant="destructive" class="py-2 px-3">
				<UiAlertDescription class="text-[11px] font-medium">
					{{ displayError }}
				</UiAlertDescription>
			</UiAlert>
		</div>

		<div
			:ref="containerProps.ref"
			:style="containerProps.style"
			class="h-[220px] overflow-y-auto"
			@scroll="handleListScroll"
		>
			<div
				v-if="loading && items.length === 0"
				class="py-8 text-center text-xs text-muted-foreground"
			>
				Loading {{ pluralLabel }}...
			</div>

			<div
				v-else-if="!loading && filteredItems.length === 0"
				class="py-8 text-center text-xs text-muted-foreground"
			>
				No {{ pluralLabel }} found matching filter
			</div>

			<div v-else v-bind="wrapperProps" class="p-1">
				<div
					v-for="{ data: item } in renderedItems"
					:key="item.id"
					class="h-9 flex items-center justify-between px-3 py-1.5 rounded-md transition-colors cursor-pointer border-b border-border/50 last:border-b-0"
					:class="
						isChecked(item.id)
							? 'bg-sky-50/80 dark:bg-sky-950/40'
							: 'hover:bg-accent/60'
					"
					@click="setChecked(item.id)"
				>
					<div class="flex items-center gap-2.5 min-w-0">
						<UiCheckbox
							:model-value="isChecked(item.id)"
							:disabled="isBusy"
							:aria-label="`Select ${singularLabel}`"
							@click.stop
							@update:model-value="(val) => setChecked(item.id, Boolean(val))"
						/>
						<span
							class="text-xs truncate"
							:class="
								isChecked(item.id)
									? 'font-semibold text-foreground'
									: 'font-normal text-foreground/80'
							"
						>
							{{ item.name }}
						</span>
					</div>

					<UiBadge
						v-if="item.badge"
						:variant="isChecked(item.id) ? 'outline' : 'secondary'"
						class="px-2 py-0.5 text-[10px] font-mono font-normal shrink-0"
						:class="[
							badgeUppercase ? 'uppercase' : '',
							isChecked(item.id)
								? 'bg-sky-100 dark:bg-sky-950 text-sky-700 dark:text-sky-300 border-sky-200 dark:border-sky-800'
								: ''
						]"
					>
						{{ item.badge }}
					</UiBadge>
				</div>
			</div>

			<div
				v-if="hasMore && !loading"
				class="py-2 text-center border-t border-border/50 bg-muted/20"
			>
				<button
					type="button"
					class="text-[11px] font-medium text-sky-600 dark:text-sky-400 hover:underline disabled:opacity-50"
					:disabled="loadingMore || isBusy"
					@click="onLoadMore && onLoadMore()"
				>
					{{
						loadingMore
							? `Loading more ${pluralLabel}...`
							: `Load more (${items.length} of ${effectiveTotal} loaded)`
					}}
				</button>
			</div>
		</div>
	</DefineSelectorBody>

	<DefineSelectorActions>
		<UiButton variant="outline" size="xs" :disabled="isBusy" @click="clearAll">
			Clear All
		</UiButton>
		<UiButton size="xs" :disabled="isBusy || loading" @click="applySelection">
			{{
				resolvingAll
					? `Fetching ${pluralLabel}...`
					: saving
						? 'Saving...'
						: `Apply Selection (${selectedCount})`
			}}
		</UiButton>
	</DefineSelectorActions>

	<UiCard v-if="inline" class="w-full overflow-hidden shadow-xl">
		<UiCardHeader
			class="flex flex-row items-center justify-between space-y-0 px-4 py-3 border-b border-border"
		>
			<div class="flex items-center gap-2">
				<span class="text-xs font-semibold text-foreground">{{ title }}</span>
				<UiBadge
					variant="secondary"
					class="px-2 py-0.5 text-[10px] font-medium bg-sky-100 dark:bg-sky-950 text-sky-700 dark:text-sky-300"
				>
					{{ selectedCount }} of {{ badgeTotal }} Selected
				</UiBadge>
			</div>
			<UiButton
				variant="ghost"
				size="icon"
				class="h-6 w-6 text-muted-foreground hover:text-foreground"
				:disabled="isBusy"
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
			@escape-key-down="(e) => isBusy && e.preventDefault()"
			@pointer-down-outside="(e) => isBusy && e.preventDefault()"
			@interact-outside="(e) => isBusy && e.preventDefault()"
		>
			<UiDialogHeader
				class="px-4 py-3 border-b border-border flex-row items-center gap-2 space-y-0 pr-10"
			>
				<UiDialogTitle class="text-xs font-semibold text-foreground">
					{{ title }}
				</UiDialogTitle>
				<UiBadge
					variant="secondary"
					class="px-2 py-0.5 text-[10px] font-medium bg-sky-100 dark:bg-sky-950 text-sky-700 dark:text-sky-300"
				>
					{{ selectedCount }} of {{ badgeTotal }} Selected
				</UiBadge>
				<UiDialogDescription class="sr-only">
					{{ description }}
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

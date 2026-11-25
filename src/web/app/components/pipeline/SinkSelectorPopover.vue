<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed } from 'vue'
import ResourceSelectorPopover from '@/components/pipeline/ResourceSelectorPopover.vue'
import type { ResourceSelectorItem } from '@/composables/useResourceSelector'
import type { ISinkSelectorItem } from '@@/shared/types/pipeline.type'

const props = withDefaults(
	defineProps<{
		modelValue?: boolean
		inline?: boolean
		suspended?: boolean
		sinks?: ISinkSelectorItem[]
		totalCount?: number
		selectedIds?: string[]
		loading?: boolean
		loadingMore?: boolean
		saving?: boolean
		errorMessage?: string | null
		onSearch?: (query: string) => Promise<void>
		onLoadMore?: () => Promise<void>
		onResolveAllIds?: (searchQuery?: string) => Promise<string[]>
	}>(),
	{
		modelValue: true,
		inline: false,
		suspended: false,
		sinks: () => [],
		totalCount: undefined,
		selectedIds: () => [],
		loading: false,
		loadingMore: false,
		saving: false,
		errorMessage: null,
		onSearch: undefined,
		onLoadMore: undefined,
		onResolveAllIds: undefined
	}
)

const emit = defineEmits<{
	(e: 'update:modelValue', val: boolean): void
	(e: 'close'): void
	(e: 'apply', ids: string[]): void
	(e: 'clearError'): void
}>()

const items = computed<ResourceSelectorItem[]>(() =>
	props.sinks.map((s) => ({
		id: s.id,
		name: s.name,
		badge: s.type
	}))
)
</script>

<template>
	<ResourceSelectorPopover
		:model-value="modelValue"
		:inline="inline"
		:suspended="suspended"
		:items="items"
		:total-count="totalCount"
		:source-ids="selectedIds"
		:loading="loading"
		:loading-more="loadingMore"
		:saving="saving"
		:error-message="errorMessage"
		title="Select Sinks"
		description="Select sinks to attach to this pipeline"
		search-placeholder="Search sink by name..."
		singular-label="sink"
		plural-label="sinks"
		:badge-uppercase="true"
		:on-search="onSearch"
		:on-load-more="onLoadMore"
		:on-resolve-all-ids="onResolveAllIds"
		@update:model-value="(val) => emit('update:modelValue', val)"
		@close="emit('close')"
		@commit="(ids) => emit('apply', ids)"
		@clear-error="emit('clearError')"
	/>
</template>

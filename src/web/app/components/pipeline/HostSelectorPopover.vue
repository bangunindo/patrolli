<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed } from 'vue'
import ResourceSelectorPopover from '@/components/pipeline/ResourceSelectorPopover.vue'
import type { ResourceSelectorItem } from '@/composables/useResourceSelector'
import type { IHostSelectorItem } from '@@/shared/types/pipeline.type'

const props = withDefaults(
	defineProps<{
		modelValue?: boolean
		inline?: boolean
		suspended?: boolean
		hosts?: IHostSelectorItem[]
		totalCount?: number
		selectedIds?: string[]
		loading?: boolean
		loadingMore?: boolean
		saving?: boolean
		minSelection?: number
		errorMessage?: string | null
		onSearch?: (query: string) => Promise<void>
		onLoadMore?: () => Promise<void>
		onResolveAllIds?: (searchQuery?: string) => Promise<string[]>
	}>(),
	{
		modelValue: true,
		inline: false,
		suspended: false,
		hosts: () => [],
		totalCount: undefined,
		selectedIds: () => [],
		loading: false,
		loadingMore: false,
		saving: false,
		minSelection: 0,
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
	props.hosts.map((h) => ({
		id: h.id,
		name: h.name,
		badge: h.ip
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
		:min-selection="minSelection"
		:error-message="errorMessage"
		title="Select Hosts"
		description="Select hosts to attach to this pipeline"
		search-placeholder="Search host by name..."
		singular-label="host"
		plural-label="hosts"
		:on-search="onSearch"
		:on-load-more="onLoadMore"
		:on-resolve-all-ids="onResolveAllIds"
		@update:model-value="(val) => emit('update:modelValue', val)"
		@close="emit('close')"
		@commit="(ids) => emit('apply', ids)"
		@clear-error="emit('clearError')"
	/>
</template>

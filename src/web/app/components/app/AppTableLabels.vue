<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed } from 'vue'

const props = withDefaults(
	defineProps<{
		labels?: unknown
		maxVisible?: number
	}>(),
	{
		labels: undefined,
		maxVisible: 2
	}
)

function parseLabels(labels: unknown): Array<[string, unknown]> {
	if (!labels) return []
	if (typeof labels === 'object' && !Array.isArray(labels)) {
		return Object.entries(labels as Record<string, unknown>)
	}
	if (typeof labels === 'string') {
		try {
			const parsed = JSON.parse(labels)
			if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) {
				return Object.entries(parsed as Record<string, unknown>)
			}
		} catch {
			return []
		}
	}
	return []
}

const labelEntries = computed(() => parseLabels(props.labels))
const visibleLabels = computed(() => labelEntries.value.slice(0, props.maxVisible))
const hiddenLabels = computed(() => labelEntries.value.slice(props.maxVisible))
</script>

<template>
	<div v-if="labelEntries.length > 0" class="flex flex-wrap items-center gap-1.5 max-w-sm">
		<UiBadge
			v-for="[key, value] in visibleLabels"
			:key="key"
			variant="outline"
			class="text-xs font-normal max-w-[180px]"
			:title="`${key} : ${value}`"
		>
			<span class="truncate">{{ key }} : {{ value }}</span>
		</UiBadge>

		<UiPopover v-if="hiddenLabels.length > 0">
			<UiPopoverTrigger as-child>
				<button
					type="button"
					class="inline-flex items-center rounded-md border border-border px-2 py-0.5 text-xs font-medium text-muted-foreground hover:bg-accent hover:text-accent-foreground transition-colors cursor-pointer"
					@click.stop
				>
					+{{ hiddenLabels.length }}
				</button>
			</UiPopoverTrigger>

			<UiPopoverContent align="start" class="w-72 p-3" @click.stop>
				<div class="text-xs font-medium text-muted-foreground mb-2">
					More Labels ({{ hiddenLabels.length }})
				</div>
				<div class="flex flex-wrap gap-1.5 max-h-48 overflow-y-auto">
					<UiBadge
						v-for="[key, value] in hiddenLabels"
						:key="key"
						variant="outline"
						class="text-xs font-normal break-all"
					>
						{{ key }} : {{ value }}
					</UiBadge>
				</div>
			</UiPopoverContent>
		</UiPopover>
	</div>
	<span v-else class="text-muted-foreground">-</span>
</template>

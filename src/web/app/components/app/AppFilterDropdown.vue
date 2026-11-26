<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { cn } from '@/app/lib/utils'

const props = withDefaults(
	defineProps<{
		active?: boolean
		showReset?: boolean
	}>(),
	{
		active: false,
		showReset: true
	}
)

const emits = defineEmits<{
	reset: []
}>()
</script>

<template>
	<UiDropdownMenu>
		<UiDropdownMenuTrigger as-child>
			<UiButton
				variant="outline"
				size="sm"
				:class="
					cn(
						'gap-1.5 font-medium',
						props.active &&
							'border-primary text-primary bg-primary/5 hover:bg-primary/10 hover:text-primary'
					)
				"
			>
				<UiIcon name="filter-outline" size="16px" />
				<span>Filter</span>
				<span
					v-if="props.active"
					class="h-2 w-2 rounded-full bg-primary shrink-0"
					aria-hidden="true"
				/>
			</UiButton>
		</UiDropdownMenuTrigger>
		<UiDropdownMenuContent
			align="end"
			class="bg-card border border-border p-4 min-w-[300px] shadow-lg rounded-xl"
		>
			<div class="flex flex-col gap-4">
				<div
					class="flex items-center justify-between border-b border-border pb-2 text-foreground"
				>
					<span class="font-semibold text-sm">Filters</span>
					<button
						v-if="props.active && props.showReset"
						type="button"
						class="text-xs font-medium text-primary hover:underline focus:outline-none"
						@click="emits('reset')"
					>
						Reset
					</button>
				</div>
				<div class="flex flex-col gap-3">
					<slot />
				</div>
			</div>
		</UiDropdownMenuContent>
	</UiDropdownMenu>
</template>

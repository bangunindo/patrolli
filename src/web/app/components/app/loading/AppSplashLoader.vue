<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
withDefaults(
	defineProps<{
		title?: string
		subtitle?: string
	}>(),
	{
		title: 'PATROLLI',
		subtitle: 'Loading application...'
	}
)
</script>

<template>
	<div
		class="fixed inset-0 flex flex-col items-center justify-center bg-white dark:bg-slate-950 text-slate-800 dark:text-slate-100 z-50 select-none"
	>
		<div class="flex flex-col items-center gap-4">
			<!-- Branded Spinner -->
			<div
				class="w-10 h-10 border-4 border-slate-200 dark:border-slate-800 border-t-sky-500 dark:border-t-sky-400 rounded-full animate-spin"
			/>
			<div class="flex flex-col items-center gap-1 text-center">
				<span
					class="text-sm font-bold tracking-widest text-slate-800 dark:text-slate-200 uppercase"
				>
					{{ title }}
				</span>
				<span class="text-xs text-slate-400 font-medium">
					{{ subtitle }}
				</span>
			</div>
		</div>
	</div>
</template>

<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts"></script>

<template>
	<UiDropdownMenu>
		<UiDropdownMenuTrigger as-child>
			<UiButton variant="outline" size="sm" class="gap-1.5 font-medium">
				<UiIcon name="filter-outline" size="16px" />
				<span>Filter</span>
			</UiButton>
		</UiDropdownMenuTrigger>
		<UiDropdownMenuContent
			align="end"
			class="bg-card border border-border p-4 min-w-[300px] shadow-lg rounded-xl"
		>
			<div class="flex flex-col gap-4">
				<div class="font-semibold text-sm border-b border-border pb-2 text-foreground">
					Filters
				</div>
				<div class="flex flex-col gap-3">
					<slot />
				</div>
			</div>
		</UiDropdownMenuContent>
	</UiDropdownMenu>
</template>

<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { type HTMLAttributes } from 'vue'
import { cn } from '@/app/lib/utils'

const props = defineProps<{
	class?: HTMLAttributes['class']
}>()
</script>

<template>
	<div
		:class="
			cn(
				'w-full bg-card text-card-foreground border border-border rounded-xl p-6 shadow-xs space-y-6',
				props.class
			)
		"
	>
		<slot />
	</div>
</template>

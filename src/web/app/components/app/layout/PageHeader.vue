<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { type HTMLAttributes } from 'vue'
import { cn } from '@/app/lib/utils'

const props = withDefaults(
	defineProps<{
		title?: string
		description?: string
		showBack?: boolean
		class?: HTMLAttributes['class']
	}>(),
	{
		title: '',
		description: '',
		showBack: false,
		class: undefined
	}
)

const emit = defineEmits<{
	back: []
}>()
</script>

<template>
	<div :class="cn('flex items-center justify-between gap-4 w-full', props.class)">
		<div class="flex items-center gap-3">
			<UiButton
				v-if="props.showBack"
				variant="ghost"
				size="icon"
				class="h-9 w-9 text-muted-foreground hover:text-foreground shrink-0"
				@click="emit('back')"
			>
				<UiIcon name="arrow-left" size="18px" />
			</UiButton>
			<div class="flex flex-col gap-0.5">
				<slot name="title">
					<h1 class="text-2xl font-bold tracking-tight text-foreground">
						{{ props.title }}
					</h1>
				</slot>
				<slot name="description">
					<p v-if="props.description" class="text-sm text-muted-foreground">
						{{ props.description }}
					</p>
				</slot>
			</div>
		</div>

		<div v-if="$slots.actions" class="flex items-center gap-2">
			<slot name="actions" />
		</div>
	</div>
</template>

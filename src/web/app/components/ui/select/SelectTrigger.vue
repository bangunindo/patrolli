<script setup lang="ts">
import type { SelectTriggerProps } from 'reka-ui'
import type { HTMLAttributes } from 'vue'
import { reactiveOmit } from '@vueuse/core'
import Icon from '@/components/ui/icon/Icon.vue'
import { SelectIcon, SelectTrigger, useForwardProps } from 'reka-ui'
import { cn } from '@/app/lib/utils'

const props = defineProps<SelectTriggerProps & { class?: HTMLAttributes['class'] }>()

const delegatedProps = reactiveOmit(props, 'class')

const forwardedProps = useForwardProps(delegatedProps)
</script>

<template>
	<SelectTrigger
		v-bind="forwardedProps"
		:class="
			cn(
				'select-trigger flex h-9 w-full items-center justify-between rounded-md border border-input bg-transparent dark:bg-slate-900/40 px-3 py-1 text-sm shadow-sm transition-colors data-[placeholder]:text-muted-foreground focus:outline-none focus:ring-1 focus:ring-ring focus:border-input disabled:cursor-not-allowed disabled:opacity-50 [&>span]:truncate text-start hover:border-slate-400 dark:hover:border-slate-600',
				props.class
			)
		"
	>
		<slot />
		<SelectIcon as-child>
			<Icon name="chevron-down" size="16px" class="opacity-50 shrink-0" />
		</SelectIcon>
	</SelectTrigger>
</template>

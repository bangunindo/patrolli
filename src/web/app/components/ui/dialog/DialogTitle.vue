<script setup lang="ts">
import { DialogTitle, type DialogTitleProps } from 'reka-ui'
import { computed, type HTMLAttributes } from 'vue'
import { cn } from '@/app/lib/utils'

const props = defineProps<DialogTitleProps & { class?: HTMLAttributes['class'] }>()

const delegatedProps = computed(() => {
	const { class: _, ...delegated } = props
	return delegated
})
</script>

<template>
	<DialogTitle
		v-bind="delegatedProps"
		:class="cn('text-lg font-semibold leading-none tracking-tight', props.class)"
	>
		<slot />
	</DialogTitle>
</template>

<script setup lang="ts">
import { AvatarFallback, type AvatarFallbackProps } from 'reka-ui'
import { computed, type HTMLAttributes } from 'vue'
import { cn } from '@/app/lib/utils'

const props = defineProps<AvatarFallbackProps & { class?: HTMLAttributes['class'] }>()

const delegatedProps = computed(() => {
	const { class: _, ...delegated } = props
	return delegated
})
</script>

<template>
	<AvatarFallback
		v-bind="delegatedProps"
		:class="
			cn(
				'flex h-full w-full items-center justify-center rounded-full bg-primary text-primary-foreground font-semibold',
				props.class
			)
		"
	>
		<slot />
	</AvatarFallback>
</template>

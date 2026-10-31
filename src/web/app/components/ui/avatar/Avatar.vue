<script setup lang="ts">
import { AvatarRoot, type AvatarRootProps } from 'reka-ui'
import { computed, type HTMLAttributes } from 'vue'
import { cn } from '@/app/lib/utils'

const props = withDefaults(
	defineProps<AvatarRootProps & { class?: HTMLAttributes['class']; size?: 'sm' | 'md' | 'lg' }>(),
	{
		size: 'md'
	}
)

const sizeMap = {
	sm: 'h-8 w-8 text-xs',
	md: 'h-10 w-10 text-sm',
	lg: 'h-12 w-12 text-base'
}

const delegatedProps = computed(() => {
	const { class: _, size: __, ...delegated } = props
	return delegated
})
</script>

<template>
	<AvatarRoot
		v-bind="delegatedProps"
		:class="
			cn(
				'relative flex shrink-0 overflow-hidden rounded-full font-medium',
				sizeMap[props.size],
				props.class
			)
		"
	>
		<slot />
	</AvatarRoot>
</template>

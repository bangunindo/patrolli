<script setup lang="ts">
import { Primitive, type PrimitiveProps } from 'reka-ui'
import { type HTMLAttributes } from 'vue'
import { cn } from '@/app/lib/utils'
import { buttonVariants, type ButtonVariants } from '.'

interface Props extends PrimitiveProps {
	variant?: ButtonVariants['variant']
	size?: ButtonVariants['size']
	class?: HTMLAttributes['class']
}

const props = withDefaults(defineProps<Props>(), {
	as: 'button',
	variant: 'default',
	size: 'default'
})
</script>

<template>
	<Primitive
		:as="props.as"
		:as-child="props.asChild"
		:class="cn(buttonVariants({ variant: props.variant, size: props.size }), props.class)"
	>
		<slot />
	</Primitive>
</template>

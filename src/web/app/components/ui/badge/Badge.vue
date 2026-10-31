<script setup lang="ts">
import { type HTMLAttributes } from 'vue'
import { cn } from '@/app/lib/utils'
import { badgeVariants, type BadgeVariants } from '.'

interface Props {
	variant?: BadgeVariants['variant']
	class?: HTMLAttributes['class']
}

const props = withDefaults(defineProps<Props>(), {
	variant: 'default'
})
</script>

<template>
	<div :class="cn(badgeVariants({ variant: props.variant }), props.class)">
		<slot />
	</div>
</template>

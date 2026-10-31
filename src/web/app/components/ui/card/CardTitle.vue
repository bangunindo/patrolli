<script setup lang="ts">
import { type HTMLAttributes } from 'vue'
import { cn } from '@/app/lib/utils'

const props = defineProps<{
	class?: HTMLAttributes['class']
}>()
</script>

<template>
	<h3 :class="cn('font-semibold leading-none tracking-tight text-base', props.class)">
		<slot />
	</h3>
</template>

<script setup lang="ts">
import { Icon as IconifyIcon } from '@iconify/vue'
import { computed, type HTMLAttributes } from 'vue'
import { cn } from '@/app/lib/utils'

interface IconProps {
	name?: string
	icon?: string
	size?: 'xs' | 'sm' | 'md' | 'lg' | 'xl' | '2xl' | number | string
	color?: string
	class?: HTMLAttributes['class']
}

const props = withDefaults(defineProps<IconProps>(), {
	size: 'md',
	color: undefined
})

const resolvedName = computed(() => {
	const raw = props.name || props.icon || ''
	if (!raw) return ''
	if (raw.includes(':')) return raw
	// If it doesn't have a prefix, default to Material Design Icons (mdi)
	return `mdi:${raw}`
})

const sizeMap: Record<string, string> = {
	xs: '14px',
	sm: '18px',
	md: '22px',
	lg: '26px',
	xl: '32px',
	'2xl': '40px'
}

const resolvedSize = computed(() => {
	if (typeof props.size === 'number') return `${props.size}px`
	if (props.size && sizeMap[props.size]) return sizeMap[props.size]
	return props.size || '22px'
})

const colorClassMap: Record<string, string> = {
	primary: 'text-primary',
	secondary: 'text-secondary-foreground',
	error: 'text-destructive',
	danger: 'text-destructive',
	warning: 'text-amber-500',
	success: 'text-emerald-500',
	green: 'text-emerald-500',
	info: 'text-sky-500',
	muted: 'text-muted-foreground'
}

const resolvedColorClass = computed(() => {
	if (!props.color) return ''
	return colorClassMap[props.color] ?? ''
})

const isCustomColor = computed(() => {
	return (
		props.color &&
		!colorClassMap[props.color] &&
		(props.color.startsWith('#') ||
			props.color.startsWith('rgb') ||
			props.color.startsWith('var'))
	)
})
</script>

<template>
	<IconifyIcon
		v-if="resolvedName"
		:icon="resolvedName"
		:width="resolvedSize"
		:height="resolvedSize"
		:class="cn('inline-block shrink-0 align-middle', resolvedColorClass, props.class)"
		:style="isCustomColor ? { color: props.color } : undefined"
		aria-hidden="true"
	/>
</template>

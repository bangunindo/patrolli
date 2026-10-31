<script setup lang="ts">
import {
	DropdownMenuItem,
	type DropdownMenuItemEmits,
	type DropdownMenuItemProps,
	useForwardPropsEmits
} from 'reka-ui'
import { computed, type HTMLAttributes } from 'vue'
import { cn } from '@/app/lib/utils'

const props = defineProps<
	DropdownMenuItemProps & { class?: HTMLAttributes['class']; inset?: boolean }
>()
const emits = defineEmits<DropdownMenuItemEmits>()

const delegatedProps = computed(() => {
	const { class: _, inset: __, ...delegated } = props
	return delegated
})

const forwarded = useForwardPropsEmits(delegatedProps, emits)
</script>

<template>
	<DropdownMenuItem
		v-bind="forwarded"
		:class="
			cn(
				'relative flex cursor-pointer select-none items-center gap-2 rounded-sm px-2 py-1.5 text-sm outline-none transition-colors focus:bg-accent focus:text-accent-foreground data-[disabled]:pointer-events-none data-[disabled]:opacity-50 [&>svg]:size-4 [&>svg]:shrink-0',
				inset && 'pl-8',
				props.class
			)
		"
	>
		<slot />
	</DropdownMenuItem>
</template>

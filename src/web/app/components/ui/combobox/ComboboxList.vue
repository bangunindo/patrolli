<script setup lang="ts">
import type { ComboboxContentEmits, ComboboxContentProps } from 'reka-ui'
import type { HTMLAttributes } from 'vue'
import { reactiveOmit } from '@vueuse/core'
import { ComboboxContent, ComboboxPortal, useForwardPropsEmits } from 'reka-ui'
import { cn } from '@/app/lib/utils'

defineOptions({
	inheritAttrs: false
})

const props = withDefaults(
	defineProps<ComboboxContentProps & { class?: HTMLAttributes['class']; portal?: boolean }>(),
	{
		position: 'popper',
		align: 'start',
		sideOffset: 4,
		portal: true
	}
)
const emits = defineEmits<ComboboxContentEmits>()

const delegatedProps = reactiveOmit(props, 'class', 'portal')
const forwarded = useForwardPropsEmits(delegatedProps, emits)
</script>

<template>
	<ComboboxPortal :disabled="!props.portal">
		<ComboboxContent
			v-bind="{ ...forwarded, ...$attrs }"
			:class="
				cn(
					'z-50 w-[--reka-combobox-trigger-width] min-w-[8rem] overflow-hidden rounded-md border bg-popover text-popover-foreground shadow-md outline-none data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95 data-[side=bottom]:slide-in-from-top-2 data-[side=left]:slide-in-from-right-2 data-[side=right]:slide-in-from-left-2 data-[side=top]:slide-in-from-bottom-2',
					props.class
				)
			"
		>
			<div role="presentation" class="max-h-72 overflow-y-auto overflow-x-hidden p-1">
				<slot />
			</div>
		</ComboboxContent>
	</ComboboxPortal>
</template>

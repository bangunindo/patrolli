<script setup lang="ts">
import { PaginationFirst, type PaginationFirstProps } from 'reka-ui'
import { computed, type HTMLAttributes } from 'vue'
import { cn } from '@/app/lib/utils'
import { buttonVariants } from '@/components/ui/button'
import Icon from '@/components/ui/icon/Icon.vue'

const props = withDefaults(
	defineProps<PaginationFirstProps & { class?: HTMLAttributes['class'] }>(),
	{
		asChild: true
	}
)

const delegatedProps = computed(() => {
	const { class: _, ...delegated } = props
	return delegated
})
</script>

<template>
	<PaginationFirst v-bind="delegatedProps">
		<button
			:class="
				cn(buttonVariants({ variant: 'outline', size: 'icon' }), 'h-8 w-8', props.class)
			"
		>
			<slot>
				<Icon name="chevron-double-left" size="16px" />
			</slot>
		</button>
	</PaginationFirst>
</template>

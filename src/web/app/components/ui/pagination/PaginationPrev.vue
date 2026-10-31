<script setup lang="ts">
import { PaginationPrev, type PaginationPrevProps } from 'reka-ui'
import { computed, type HTMLAttributes } from 'vue'
import { cn } from '@/app/lib/utils'
import { buttonVariants } from '@/components/ui/button'
import Icon from '@/components/ui/icon/Icon.vue'

const props = withDefaults(
	defineProps<PaginationPrevProps & { class?: HTMLAttributes['class'] }>(),
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
	<PaginationPrev v-bind="delegatedProps">
		<button
			:class="
				cn(buttonVariants({ variant: 'outline', size: 'icon' }), 'h-8 w-8', props.class)
			"
		>
			<slot>
				<Icon name="chevron-left" size="16px" />
			</slot>
		</button>
	</PaginationPrev>
</template>

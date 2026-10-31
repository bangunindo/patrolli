<script setup lang="ts">
import { PaginationEllipsis, type PaginationEllipsisProps } from 'reka-ui'
import { computed, type HTMLAttributes } from 'vue'
import { cn } from '@/app/lib/utils'
import Icon from '@/components/ui/icon/Icon.vue'

const props = defineProps<PaginationEllipsisProps & { class?: HTMLAttributes['class'] }>()

const delegatedProps = computed(() => {
	const { class: _, ...delegated } = props
	return delegated
})
</script>

<template>
	<PaginationEllipsis
		v-bind="delegatedProps"
		:class="cn('flex h-8 w-8 items-center justify-center', props.class)"
	>
		<slot>
			<Icon name="dots-horizontal" size="16px" />
		</slot>
	</PaginationEllipsis>
</template>

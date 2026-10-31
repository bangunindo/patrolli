<script setup lang="ts">
import {
	PaginationRoot,
	type PaginationRootEmits,
	type PaginationRootProps,
	useForwardPropsEmits
} from 'reka-ui'

const props = defineProps<PaginationRootProps>()
const emits = defineEmits<PaginationRootEmits>()

const forwarded = useForwardPropsEmits(props, emits)
</script>

<template>
	<PaginationRoot v-bind="forwarded">
		<slot />
	</PaginationRoot>
</template>

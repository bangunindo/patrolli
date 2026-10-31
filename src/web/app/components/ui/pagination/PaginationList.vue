<script setup lang="ts">
import { PaginationList, type PaginationListProps } from 'reka-ui'

const props = defineProps<PaginationListProps>()
</script>

<template>
	<PaginationList v-slot="{ items }" v-bind="props" class="flex items-center gap-1">
		<slot :items="items" />
	</PaginationList>
</template>

<script setup lang="ts">
import { PaginationListItem, type PaginationListItemProps } from 'reka-ui'

const props = defineProps<PaginationListItemProps>()
</script>

<template>
	<PaginationListItem v-bind="props">
		<slot />
	</PaginationListItem>
</template>

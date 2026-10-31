<script setup lang="ts">
import type { SelectScrollUpButtonProps } from 'reka-ui'
import type { HTMLAttributes } from 'vue'
import { reactiveOmit } from '@vueuse/core'
import Icon from '@/components/ui/icon/Icon.vue'
import { SelectScrollUpButton, useForwardProps } from 'reka-ui'
import { cn } from '@/app/lib/utils'

const props = defineProps<SelectScrollUpButtonProps & { class?: HTMLAttributes['class'] }>()

const delegatedProps = reactiveOmit(props, 'class')

const forwardedProps = useForwardProps(delegatedProps)
</script>

<template>
	<SelectScrollUpButton
		v-bind="forwardedProps"
		:class="cn('flex cursor-default items-center justify-center py-1', props.class)"
	>
		<slot>
			<Icon name="chevron-up" size="16px" />
		</slot>
	</SelectScrollUpButton>
</template>

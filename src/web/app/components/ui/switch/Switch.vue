<script setup lang="ts">
import {
	SwitchRoot,
	type SwitchRootEmits,
	type SwitchRootProps,
	SwitchThumb,
	useForwardPropsEmits
} from 'reka-ui'
import { computed, type HTMLAttributes } from 'vue'
import { cn } from '@/app/lib/utils'

const props = withDefaults(
	defineProps<
		SwitchRootProps & {
			class?: HTMLAttributes['class']
			checked?: boolean
		}
	>(),
	{
		checked: undefined
	}
)
const emits = defineEmits<
	SwitchRootEmits & {
		'update:checked': [value: boolean]
	}
>()

const resolvedModelValue = computed<boolean | undefined>(() => {
	if (props.checked !== undefined) {
		return props.checked
	}
	return props.modelValue ?? undefined
})

const delegatedProps = computed(() => {
	const { class: _, checked: __, modelValue: ___, ...delegated } = props
	return delegated
})

const forwarded = useForwardPropsEmits(delegatedProps, emits)

const handleModelValueUpdate = (val: boolean) => {
	emits('update:checked', val)
	emits('update:modelValue', val)
}
</script>

<template>
	<SwitchRoot
		v-bind="forwarded"
		:model-value="resolvedModelValue"
		@update:model-value="handleModelValueUpdate"
		:class="
			cn(
				'peer inline-flex h-5 w-9 shrink-0 cursor-pointer items-center rounded-full border-2 border-transparent shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background disabled:cursor-not-allowed disabled:opacity-50 data-[state=checked]:bg-primary data-[state=unchecked]:bg-input',
				props.class
			)
		"
	>
		<SwitchThumb
			:class="
				cn(
					'pointer-events-none block h-4 w-4 rounded-full bg-background shadow-lg ring-0 transition-transform data-[state=checked]:translate-x-4 data-[state=unchecked]:translate-x-0'
				)
			"
		>
			<slot name="thumb" />
		</SwitchThumb>
	</SwitchRoot>
</template>

<script setup lang="ts">
import {
	CheckboxIndicator,
	CheckboxRoot,
	type CheckboxRootEmits,
	type CheckboxRootProps,
	useForwardPropsEmits
} from 'reka-ui'
import { computed, type HTMLAttributes } from 'vue'
import { cn } from '@/app/lib/utils'
import Icon from '@/components/ui/icon/Icon.vue'

const props = withDefaults(
	defineProps<
		CheckboxRootProps & {
			class?: HTMLAttributes['class']
			checked?: boolean | 'indeterminate'
		}
	>(),
	{
		checked: undefined
	}
)
const emits = defineEmits<
	CheckboxRootEmits & {
		'update:checked': [value: boolean | 'indeterminate']
	}
>()

const resolvedModelValue = computed<boolean | 'indeterminate' | undefined>(() => {
	if (props.checked !== undefined) {
		return props.checked
	}
	return props.modelValue
})

const delegatedProps = computed(() => {
	const { class: _, checked: __, modelValue: ___, ...delegated } = props
	return delegated
})

const forwarded = useForwardPropsEmits(delegatedProps, emits)

const handleModelValueUpdate = (val: boolean | 'indeterminate') => {
	emits('update:checked', val)
	emits('update:modelValue', val)
}
</script>

<template>
	<CheckboxRoot
		v-bind="forwarded"
		:model-value="resolvedModelValue"
		@update:model-value="handleModelValueUpdate"
		:class="
			cn(
				'peer h-4 w-4 shrink-0 rounded-[4px] border border-[#7E8186] dark:border-slate-500 bg-white dark:bg-slate-900 shadow-sm transition-colors duration-150',
				'hover:border-primary focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring focus-visible:ring-offset-1',
				'disabled:cursor-not-allowed disabled:opacity-50',
				'data-[state=checked]:bg-primary data-[state=checked]:border-primary data-[state=checked]:text-primary-foreground',
				'data-[state=indeterminate]:bg-primary data-[state=indeterminate]:border-primary data-[state=indeterminate]:text-primary-foreground',
				props.class
			)
		"
	>
		<template #default="{ state }">
			<CheckboxIndicator class="flex h-full w-full items-center justify-center text-current">
				<slot :state="state">
					<Icon v-if="state === 'indeterminate'" name="minus" size="14px" />
					<Icon v-else name="check" size="14px" />
				</slot>
			</CheckboxIndicator>
		</template>
	</CheckboxRoot>
</template>

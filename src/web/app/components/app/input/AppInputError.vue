<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import UiIcon from '@/components/ui/icon/Icon.vue'

const props = withDefaults(
	defineProps<{
		modelValue?: boolean
		messages?: string | string[] | null
		error?: string | string[] | null
		closeable?: boolean
	}>(),
	{
		modelValue: true,
		messages: () => [],
		error: () => [],
		closeable: true
	}
)

const emits = defineEmits<{
	'update:modelValue': [val: boolean]
	close: []
}>()

const isDismissed = ref(false)

const rawMessages = computed(() => {
	if (
		props.messages &&
		(Array.isArray(props.messages) ? props.messages.length > 0 : Boolean(props.messages))
	) {
		return props.messages
	}
	if (
		props.error &&
		(Array.isArray(props.error) ? props.error.length > 0 : Boolean(props.error))
	) {
		return props.error
	}
	return []
})

const messageList = computed<string[]>(() => {
	const raw = rawMessages.value
	if (!raw) return []
	if (Array.isArray(raw)) {
		return raw.filter((m): m is string => typeof m === 'string' && m.trim().length > 0)
	}
	if (typeof raw === 'string' && raw.trim().length > 0) {
		return [raw.trim()]
	}
	return []
})

// Reset dismissed state when error messages change so new errors remain visible
watch(
	[() => props.messages, () => props.error],
	() => {
		isDismissed.value = false
	},
	{ deep: true }
)

// Reset dismissed state when modelValue is re-enabled by the parent component
watch(
	() => props.modelValue,
	(val) => {
		if (val) {
			isDismissed.value = false
		}
	}
)

const isShowing = computed({
	get: () => !isDismissed.value && props.modelValue && messageList.value.length > 0,
	set: (val: boolean) => {
		isDismissed.value = !val
		emits('update:modelValue', val)
		if (!val) {
			emits('close')
		}
	}
})

const errorTitle = computed(() => messageList.value.join(' | '))

const handleClose = () => {
	isShowing.value = false
}
</script>

<template>
	<div
		v-if="messageList.length > 0 && isShowing"
		class="mt-2 rounded-md border border-rose-200 bg-rose-50 dark:border-rose-900/60 dark:bg-rose-950/40 p-2.5 text-xs text-rose-800 dark:text-rose-200 flex items-center justify-between gap-2"
	>
		<span>{{ errorTitle }}</span>
		<button
			v-if="props.closeable"
			type="button"
			class="text-rose-500 hover:text-rose-700 dark:hover:text-rose-300"
			@click="handleClose"
		>
			<UiIcon name="close" class="w-3.5 h-3.5" />
		</button>
	</div>
</template>

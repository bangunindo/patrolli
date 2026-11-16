<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import type { FieldDescriptor } from '@@/shared/types/dynamic-form.type'
import DynamicFieldRenderer from '../DynamicFieldRenderer.vue'

defineProps<{
	label?: string
	variant?: 'card' | 'section'
	hideBorder?: boolean
	fields: FieldDescriptor[]
}>()
</script>

<template>
	<div
		:class="[
			variant === 'section' || hideBorder
				? 'flex flex-col gap-1.5'
				: 'border border-gray-200 dark:border-gray-700 rounded-lg p-4 bg-white dark:bg-[#1e1e1e] shadow-sm flex flex-col gap-3'
		]"
	>
		<h3
			v-if="label"
			:class="[
				variant === 'section' || hideBorder
					? 'text-xs font-bold uppercase tracking-wider text-[#0284C7] mb-0'
					: 'text-base font-semibold text-gray-800 dark:text-gray-200 border-b border-gray-200 dark:border-gray-700 pb-2 mb-1'
			]"
		>
			{{ label }}
		</h3>
		<div v-for="field in fields" :key="field.pathKey">
			<DynamicFieldRenderer :descriptor="field" :path="field.pathKey" />
		</div>
	</div>
</template>

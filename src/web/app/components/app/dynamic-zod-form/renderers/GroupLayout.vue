<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { rendererProps, useJsonFormsLayout, DispatchRenderer } from '@jsonforms/vue'
import type { GroupLayout } from '@jsonforms/core'

const props = defineProps({
	...rendererProps<GroupLayout>()
})

const { layout } = useJsonFormsLayout(props)
</script>

<template>
	<div
		v-if="layout.visible"
		:class="[
			layout.uischema.options?.variant === 'section' || layout.uischema.options?.hideBorder
				? 'flex flex-col gap-1.5'
				: 'border border-gray-200 dark:border-gray-700 rounded-lg p-4 bg-white dark:bg-[#1e1e1e] shadow-sm flex flex-col gap-3'
		]"
	>
		<h3
			v-if="layout.label"
			:class="[
				layout.uischema.options?.variant === 'section' ||
				layout.uischema.options?.hideBorder
					? 'text-xs font-bold uppercase tracking-wider text-[#0284C7] mb-0'
					: 'text-base font-semibold text-gray-800 dark:text-gray-200 border-b border-gray-200 dark:border-gray-700 pb-2 mb-1'
			]"
		>
			{{ layout.label }}
		</h3>
		<div v-for="(element, index) in layout.uischema.elements" :key="`${layout.path}-${index}`">
			<dispatch-renderer
				:schema="layout.schema"
				:uischema="element"
				:path="layout.path"
				:enabled="layout.enabled"
				:renderers="layout.renderers"
			/>
		</div>
	</div>
</template>

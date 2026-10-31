<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { rendererProps, useJsonFormsLayout, DispatchRenderer } from '@jsonforms/vue'
import type { Layout } from '@jsonforms/core'

const props = defineProps({
	...rendererProps<Layout>()
})

const { layout } = useJsonFormsLayout(props)
</script>

<template>
	<div v-if="layout.visible" class="flex flex-col gap-4">
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

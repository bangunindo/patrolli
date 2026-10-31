<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
const props = withDefaults(
	defineProps<{
		containerHeight?: string
		size?: string
	}>(),
	{
		containerHeight: '250px',
		size: '32px'
	}
)
</script>

<template>
	<div :style="{ height: containerHeight }" class="w-full flex justify-center items-center">
		<div
			:style="{ height: size, width: size }"
			class="animate-spin rounded-full border-4 border-slate-200 border-t-blue-600"
		></div>
	</div>
</template>

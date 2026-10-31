<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
const props = withDefaults(defineProps<{ label?: string }>(), {})
</script>

<template>
	<fieldset class="field-group">
		<legend class="field-group-label">{{ label }}</legend>

		<section class="px-2">
			<slot></slot>
		</section>
	</fieldset>
</template>

<style scoped>
.field-group {
	border: 1px solid #cbd5e1;
	padding: 6px 12px 14px 12px;
	border-radius: 6px;
	background: transparent;
	margin-top: 0;
	margin-bottom: 8px;
}

.field-group .field-group-label {
	font-weight: bold;
	color: var(--text-color);
	padding: 0 8px;
	font-size: 12px;
	text-transform: uppercase;
	letter-spacing: 0.5px;
}
</style>

<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
definePageMeta({
	layout: 'configuration',
	requiresAuth: true,
	middleware: [
		() => {
			return navigateTo('/configuration/host')
		}
	]
})
</script>

<template>
	<div />
</template>

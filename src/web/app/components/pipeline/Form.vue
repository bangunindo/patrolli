<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import FormContent from '@/components/pipeline/form/FormContent.vue'
import StepperSidebar from '@/components/pipeline/form/StepperSidebar.vue'
</script>

<template>
	<div class="flex flex-col md:flex-row w-full bg-white dark:bg-slate-900 overflow-hidden">
		<StepperSidebar />
		<FormContent />
	</div>
</template>

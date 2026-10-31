<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import ConfigNav from '@/components/app/navigation/ConfigNav.vue'
</script>

<template>
	<div class="configuration-layout-wrapper w-full">
		<NuxtLayout name="default">
			<div class="configuration-layout w-full">
				<!-- Sticky Navigation Bar pinned at top of #main-content -->
				<div
					class="configuration-nav-sticky sticky top-0 z-30 bg-[#F8FAFC]/90 dark:bg-background/90 backdrop-blur-sm py-3 transition-all"
				>
					<ConfigNav />
				</div>
				<div class="px-4 md:px-6 pb-8 flex gap-4 w-full items-start min-w-0">
					<slot />
				</div>
			</div>
		</NuxtLayout>
	</div>
</template>

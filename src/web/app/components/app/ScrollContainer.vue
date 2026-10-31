<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<template>
	<div ref="scrollContainer" class="scroll-container" :style="{ maxHeight: maxHeight }">
		<slot></slot>

		<!-- Read-only Loading Indicator using Props -->
		<div v-if="isFetching" class="loading">Loading...</div>
	</div>
</template>

<script setup lang="ts">
import { useElementVisibility, useInfiniteScroll } from '@vueuse/core'
import { nextTick, onMounted, ref, watch } from 'vue'

// Props and Emits declaration (update:isFetching removed)
const props = defineProps<{
	pagination?: any
	maxHeight: string
	isFetching?: boolean
}>()

const emits = defineEmits(['infinite'])

const scrollContainer = ref<HTMLElement | null>(null)
const isCooldown = ref(false)
const lastScrollHeight = ref(0)
const lastTriggeredPage = ref(0)

// Isolation feature: Detect if component is currently visible in viewport
const isVisible = useElementVisibility(scrollContainer)

// 1. CONDITION A: Auto-Load ONLY when no scrollbar exists yet
const checkAndFillContainer = async () => {
	await nextTick()
	const el = scrollContainer.value

	if (
		!el ||
		props.isFetching ||
		isCooldown.value ||
		props.pagination?.reachEnd ||
		el.clientHeight === 0
	)
		return

	const hasOverflow = el.scrollHeight > el.clientHeight

	if (!hasOverflow) {
		// Guard: Stop if DOM height did not increase after the last fetch
		if (el.scrollHeight === lastScrollHeight.value && lastScrollHeight.value > 0) {
			return
		}

		lastScrollHeight.value = el.scrollHeight
		loadMoreData()
	}
}

// 2. CONDITION B: Load More ONLY when scrolled to the very bottom
useInfiniteScroll(
	scrollContainer,
	() => {
		if (props.isFetching || isCooldown.value || props.pagination?.reachEnd || !isVisible.value)
			return

		const el = scrollContainer.value
		if (!el) return

		const hasOverflow = el.scrollHeight > el.clientHeight

		if (hasOverflow) {
			loadMoreData()
		}
	},
	{ distance: 0 }
)

// 3. WATCH ISFETCHING PROP: When loading data from parent finishes
watch(
	() => props.isFetching,
	async (newFetchingStatus) => {
		// When loading finishes (changes from true to false)
		if (!newFetchingStatus) {
			await nextTick()

			isCooldown.value = true
			setTimeout(() => {
				isCooldown.value = false
				checkAndFillContainer()
			}, 150)
		}
	}
)

// 4. WATCH DATA RESET
watch(
	() => props.pagination?.currentPage,
	(newPage) => {
		// When parent resets page back to start
		if (!newPage || newPage <= 1) {
			lastTriggeredPage.value = 0
			lastScrollHeight.value = 0
			isCooldown.value = false
			checkAndFillContainer()
		}
	}
)

// 5. WATCH VISIBILITY
watch(isVisible, (visible) => {
	if (visible) {
		checkAndFillContainer()
	}
})

onMounted(() => {
	checkAndFillContainer()
})

const loadMoreData = () => {
	if (!props.pagination || props.pagination.reachEnd || props.isFetching || isCooldown.value)
		return

	const nextPage = props.pagination.currentPage + 1

	// Guard against duplicate page fetches (Crucial since we no longer lock via emit isFetching)
	if (nextPage <= lastTriggeredPage.value) return
	lastTriggeredPage.value = nextPage

	// Send payload to parent to trigger API fetch
	const payload = {
		page: nextPage,
		limit: props.pagination.rows,
		offset: (nextPage - 1) * props.pagination.rows
	}
	emits('infinite', payload)
}
</script>

<style scoped>
.scroll-container {
	overflow-y: auto;
	display: block;
}
.loading {
	text-align: center;
	padding: 10px;
}
</style>

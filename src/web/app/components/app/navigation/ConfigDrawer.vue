<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { useMediaQuery, useScrollLock } from '@vueuse/core'
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'

defineOptions({
	name: 'config-drawer',
	alias: 'ConfigDrawer'
})

const props = withDefaults(
	defineProps<{
		modelValue?: boolean
		title?: string
		lastUpdatedText?: string
	}>(),
	{
		modelValue: false,
		title: 'Item Statistics',
		lastUpdatedText: 'Last updated: Just now'
	}
)

const emits = defineEmits<{
	(e: 'update:modelValue', value: boolean): void
	(e: 'close'): void
}>()

const closeDrawer = () => {
	emits('update:modelValue', false)
	emits('close')
}

const isDesktop = useMediaQuery('(min-width: 1280px)')
const isMounted = ref(false)

const isLocked = useScrollLock(typeof document !== 'undefined' ? document.body : null)

watch(
	[() => props.modelValue, isDesktop],
	([isOpen, desktop]) => {
		if (!desktop && isOpen) {
			isLocked.value = true
		} else {
			isLocked.value = false
		}
	},
	{ immediate: true }
)

const handleKeyDown = (e: KeyboardEvent) => {
	if (e.key === 'Escape' && props.modelValue) {
		closeDrawer()
	}
}

onMounted(() => {
	isMounted.value = true
	if (typeof window !== 'undefined') {
		window.addEventListener('keydown', handleKeyDown)
	}
})

onUnmounted(() => {
	if (typeof window !== 'undefined') {
		window.removeEventListener('keydown', handleKeyDown)
	}
	isLocked.value = false
})

const drawerClasses = computed(() => {
	if (!isDesktop.value) {
		return 'fixed inset-y-0 right-0 z-50 flex flex-col w-full sm:w-[440px] max-w-full h-full bg-card shadow-2xl border-l border-border transition-transform duration-300 ease-out'
	}
	return 'w-[380px] 2xl:w-[420px] shrink-0 rounded-2xl border border-border bg-card shadow-sm flex flex-col h-[calc(100vh-170px)] sticky top-4'
})
</script>

<template>
	<Teleport to="body" :disabled="!isMounted || isDesktop">
		<div v-if="props.modelValue" :class="isDesktop ? 'shrink-0 sticky top-4 z-20' : 'contents'">
			<!-- Backdrop for mobile/tablet screens (< xl) -->
			<div
				v-if="!isDesktop"
				class="fixed inset-0 bg-slate-900/50 backdrop-blur-xs z-40 transition-opacity duration-300"
				aria-hidden="true"
				@click="closeDrawer"
			/>

			<!-- Drawer / Sidebar container -->
			<aside
				:class="drawerClasses"
				role="dialog"
				:aria-modal="!isDesktop"
				:aria-label="props.title"
			>
				<!-- Drawer Header -->
				<div
					class="flex items-center justify-between px-4 sm:px-5 py-3.5 sm:py-4 border-b border-border shrink-0 bg-card rounded-t-2xl"
				>
					<slot name="header">
						<h3 class="mb-0 text-base font-bold text-foreground truncate">
							{{ props.title }}
						</h3>
					</slot>
					<UiButton
						variant="ghost"
						size="icon"
						class="b-button h-8 w-8 text-muted-foreground hover:text-foreground"
						@click="closeDrawer"
					>
						<UiIcon name="close" size="18px" />
					</UiButton>
				</div>

				<!-- Drawer Body -->
				<div class="flex-1 overflow-y-auto p-4 sm:p-5 min-h-0">
					<slot>
						<slot name="content" />
					</slot>
				</div>

				<!-- Drawer Footer -->
				<div
					class="px-4 sm:px-5 py-3 border-t border-border flex items-center justify-between shrink-0 rounded-b-2xl bg-muted/40"
				>
					<slot name="footer">
						<div class="flex items-center gap-1.5 text-xs text-muted-foreground">
							<UiIcon name="refresh" size="16px" />
							<span>{{ props.lastUpdatedText }}</span>
						</div>
						<UiButton variant="outline" size="sm" @click="closeDrawer">
							Close
						</UiButton>
					</slot>
				</div>
			</aside>
		</div>
	</Teleport>
</template>

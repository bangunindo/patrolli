<script lang="ts" setup>
import { Toaster as Sonner, type ToasterProps } from 'vue-sonner'
import { useAppStore } from '@/stores/app.store'

const props = withDefaults(defineProps<ToasterProps>(), {
	position: 'top-right',
	closeButton: true,
	richColors: true
})

const appStore = useAppStore()
</script>

<template>
	<Sonner
		:theme="appStore.isDarkMode ? 'dark' : 'light'"
		class="toaster group"
		v-bind="props"
		:toast-options="{
			classes: {
				toast: 'group toast group-[.toaster]:bg-card group-[.toaster]:text-foreground group-[.toaster]:border-border group-[.toaster]:shadow-xl group-[.toaster]:rounded-xl',
				description: 'group-[.toast]:text-muted-foreground',
				actionButton: 'group-[.toast]:bg-primary group-[.toast]:text-primary-foreground',
				cancelButton: 'group-[.toast]:bg-muted group-[.toast]:text-muted-foreground'
			}
		}"
	/>
</template>

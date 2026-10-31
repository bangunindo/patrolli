<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import appHelper from '@/app/helpers/app.helper'
import tokenHelper from '@/app/helpers/token.helper'
import { useAppStore } from '@/stores/app.store'
import { useAuthStore } from '@/stores/auth.store'
import { storeToRefs } from 'pinia'

const modelValue = defineModel<string[]>({
	default: () => []
})

const authStore = useAuthStore()
const appStore = useAppStore()
const { isClusterReadonly } = storeToRefs(authStore)
const { activeClusterURL } = storeToRefs(appStore)

const isOpen = ref(false)

const toggleDropdown = () => {
	isOpen.value = !isOpen.value
}

const availableClusters = computed(() => {
	const clusters = appHelper.getBaseURLs()
	const allClusters =
		clusters && clusters.length > 0
			? clusters.map((c) => ({
					label: c.label || c.name || c.value,
					value: c.value
				}))
			: [
					{ label: 'Cluster 1', value: 'Cluster 1' },
					{ label: 'Cluster 2', value: 'Cluster 2' },
					{ label: 'Cluster 3', value: 'Cluster 3' }
				]

	// 1. If user logs in with single cluster (authProvider === 'local' / isClusterReadonly)
	if (isClusterReadonly.value) {
		const active = activeClusterURL.value || appHelper.getDefaultClusterValue()
		if (active) {
			const clean = active.replace(/^\//, '')
			const matched = allClusters.find(
				(c) =>
					c.value === active ||
					c.value === clean ||
					c.label === active ||
					c.label === clean
			)
			if (matched) return [matched]
			return [{ label: appHelper.getClusterName(active) || clean, value: active }]
		}
	}

	// 2. Explicit cluster constraints from token (if present)
	const decoded = tokenHelper.decodeToken() as any
	if (decoded?.clusters && Array.isArray(decoded.clusters) && decoded.clusters.length > 0) {
		const matched = allClusters.filter((c) =>
			decoded.clusters.some(
				(dc: string) => dc === c.value || dc.replace(/^\//, '') === c.value
			)
		)
		if (matched.length > 0) return matched
	} else if (decoded?.cluster && typeof decoded.cluster === 'string') {
		const clean = decoded.cluster.replace(/^\//, '')
		const matched = allClusters.find((c) => c.value === decoded.cluster || c.value === clean)
		if (matched) return [matched]
	}

	return allClusters
})

// Auto-select when only 1 cluster is available for single-cluster user
watch(
	availableClusters,
	(clusters) => {
		if (clusters.length === 1 && clusters[0]?.value) {
			if (!modelValue.value.includes(clusters[0].value)) {
				modelValue.value = [clusters[0].value]
			}
		}
	},
	{ immediate: true }
)

const isClusterSelected = (clusterValue: string) => {
	return modelValue.value.includes(clusterValue)
}

const allSelected = computed({
	get: () => {
		return (
			availableClusters.value.length > 0 &&
			availableClusters.value.every((c) => isClusterSelected(c.value))
		)
	},
	set: (val: boolean) => {
		if (val) {
			modelValue.value = availableClusters.value.map((c) => c.value)
		} else {
			modelValue.value = []
		}
	}
})

const toggleCluster = (clusterValue: string, checked: boolean) => {
	if (checked) {
		if (!modelValue.value.includes(clusterValue)) {
			modelValue.value = [...modelValue.value, clusterValue]
		}
	} else {
		modelValue.value = modelValue.value.filter((val) => val !== clusterValue)
	}
}

const triggerLabel = computed(() => {
	if (modelValue.value.length === 0) return 'Select Clusters'
	if (availableClusters.value.length === 1 && modelValue.value.length === 1) {
		return availableClusters.value[0]?.label || '1 Cluster Selected'
	}
	if (modelValue.value.length === availableClusters.value.length) return 'All Clusters Selected'
	if (modelValue.value.length === 1) return '1 Cluster Selected'
	return `${modelValue.value.length} Clusters Selected`
})
</script>

<template>
	<div class="flex flex-col gap-1.5 pt-1">
		<!-- Field Label -->
		<label class="text-sm font-semibold text-gray-900 dark:text-gray-100">
			Target Clusters
		</label>

		<!-- Trigger Box & Collapsible Inline Panel -->
		<div class="flex flex-col gap-2">
			<button
				type="button"
				@click="toggleDropdown"
				class="w-full min-h-[42px] px-3.5 flex items-center justify-between rounded-md bg-card text-foreground border border-input text-sm text-left shadow-xs transition hover:border-slate-400 dark:hover:border-slate-600 focus:outline-none focus:ring-1 focus:ring-ring"
			>
				<span
					:class="
						modelValue.length === 0
							? 'text-muted-foreground'
							: 'text-foreground font-medium'
					"
				>
					{{ triggerLabel }}
				</span>
				<UiIcon
					:name="isOpen ? 'chevron-up' : 'chevron-down'"
					class="w-4 h-4 text-muted-foreground"
				/>
			</button>

			<!-- Collapsible Inline Panel -->
			<transition
				enter-active-class="transition-all duration-200 ease-out"
				enter-from-class="opacity-0 -translate-y-1"
				enter-to-class="opacity-100 translate-y-0"
				leave-active-class="transition-all duration-150 ease-in"
				leave-from-class="opacity-100 translate-y-0"
				leave-to-class="opacity-0 -translate-y-1"
			>
				<div
					v-if="isOpen"
					class="p-3 bg-popover text-popover-foreground border border-border rounded-md shadow-xs flex flex-col gap-2.5"
				>
					<!-- Select All Clusters (only shown when multiple clusters exist) -->
					<template v-if="availableClusters.length > 1">
						<label
							class="flex items-center gap-2 cursor-pointer text-sm font-semibold text-foreground"
						>
							<input
								type="checkbox"
								v-model="allSelected"
								class="w-4 h-4 rounded text-primary focus:ring-primary border-input"
							/>
							<span>Select All Clusters</span>
						</label>

						<!-- Divider Line -->
						<div class="border-t border-border my-0.5"></div>
					</template>

					<!-- Individual Clusters without roles -->
					<div class="flex flex-col gap-1.5">
						<div
							v-for="cluster in availableClusters"
							:key="cluster.value"
							class="flex items-center justify-between gap-3 py-2 px-2.5 rounded-md hover:bg-accent/60"
						>
							<label
								class="flex items-center gap-2 cursor-pointer text-sm font-medium text-foreground flex-1"
							>
								<input
									type="checkbox"
									:checked="isClusterSelected(cluster.value)"
									@change="
										(e: any) => toggleCluster(cluster.value, e.target.checked)
									"
									class="w-4 h-4 rounded text-primary focus:ring-primary border-input"
								/>
								<span>{{ cluster.label }}</span>
							</label>
						</div>
					</div>
				</div>
			</transition>
		</div>
	</div>
</template>

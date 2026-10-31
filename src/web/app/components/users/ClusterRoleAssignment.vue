<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import { computed, ref } from 'vue'
import type { ClusterRoleName, IClusterRoleAssignment } from '@@/shared/types/user.type'

const AVAILABLE_CLUSTERS = ['Cluster 1', 'Cluster 2', 'Cluster 3']
const ROLES: ClusterRoleName[] = ['Admin', 'Editor', 'Viewer']

const modelValue = defineModel<IClusterRoleAssignment[]>({
	default: () => []
})

const isOpen = ref(false)

const toggleDropdown = () => {
	isOpen.value = !isOpen.value
}

const isClusterSelected = (cluster: string) => {
	return modelValue.value.some((c) => c.cluster === cluster)
}

const getClusterRole = (cluster: string): ClusterRoleName => {
	const found = modelValue.value.find((c) => c.cluster === cluster)
	return found?.role || 'Viewer'
}

const allSelected = computed({
	get: () => {
		return (
			AVAILABLE_CLUSTERS.length > 0 && AVAILABLE_CLUSTERS.every((c) => isClusterSelected(c))
		)
	},
	set: (val: boolean) => {
		if (val) {
			modelValue.value = AVAILABLE_CLUSTERS.map((c) => ({
				cluster: c,
				role: getClusterRole(c)
			}))
		} else {
			modelValue.value = []
		}
	}
})

const toggleCluster = (cluster: string, checked: boolean) => {
	if (checked) {
		if (!isClusterSelected(cluster)) {
			modelValue.value = [...modelValue.value, { cluster, role: 'Viewer' }]
		}
	} else {
		modelValue.value = modelValue.value.filter((c) => c.cluster !== cluster)
	}
}

const setRole = (cluster: string, role: ClusterRoleName) => {
	const idx = modelValue.value.findIndex((c) => c.cluster === cluster)
	if (idx !== -1) {
		const updated = [...modelValue.value]
		updated[idx] = { cluster, role }
		modelValue.value = updated
	} else {
		modelValue.value = [...modelValue.value, { cluster, role }]
	}
}

const triggerLabel = computed(() => {
	if (modelValue.value.length === 0) return 'Select Clusters'
	if (modelValue.value.length === AVAILABLE_CLUSTERS.length) return 'All Clusters Selected'
	if (modelValue.value.length === 1) return '1 Cluster Selected'
	return `${modelValue.value.length} Clusters Selected`
})
</script>

<template>
	<div class="flex flex-col gap-1.5 pt-1">
		<!-- Field Label -->
		<label class="text-sm font-semibold text-gray-900 dark:text-gray-100">
			Cluster Assignment
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
					<!-- Select All Clusters -->
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

					<!-- Individual Clusters with inline Role select -->
					<div class="flex flex-col gap-1.5">
						<div
							v-for="cluster in AVAILABLE_CLUSTERS"
							:key="cluster"
							class="flex items-center justify-between gap-3 py-2 px-2.5 rounded-md hover:bg-accent/60"
						>
							<label
								class="flex items-center gap-2 cursor-pointer text-sm font-medium text-foreground"
							>
								<input
									type="checkbox"
									:checked="isClusterSelected(cluster)"
									@change="(e: any) => toggleCluster(cluster, e.target.checked)"
									class="w-4 h-4 rounded text-primary focus:ring-primary border-input"
								/>
								<span>{{ cluster }}</span>
							</label>

							<div
								v-if="isClusterSelected(cluster)"
								class="flex items-center gap-1.5"
							>
								<span class="text-[11px] text-muted-foreground">Role:</span>
								<select
									:value="getClusterRole(cluster)"
									@change="
										(e: any) =>
											setRole(cluster, e.target.value as ClusterRoleName)
									"
									class="text-xs h-7 px-2 rounded border border-input bg-card text-foreground focus:outline-none focus:ring-1 focus:ring-ring"
								>
									<option v-for="r in ROLES" :key="r" :value="r">
										{{ r }}
									</option>
								</select>
							</div>
						</div>
					</div>
				</div>
			</transition>
		</div>
	</div>
</template>

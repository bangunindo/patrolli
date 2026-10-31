<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

<script setup lang="ts">
import ModalTable from '@/components/cluster/ModalTable.vue'
import { useClusterStore } from '@/stores/cluster.store'
import appConstant from '@@/shared/constants/app.constant'
import { computed } from 'vue'
import { useRoute } from 'vue-router'

const clusterStore = useClusterStore()
const route = useRoute()

const menuList = computed(() => {
	return appConstant.menuNavConfiguration
})

const activePath = computed(() => {
	const path = route.path.split('/').filter((item) => item != '')

	return path[1] ?? 'dashboard'
})

const onClusterClick = () => {
	clusterStore.toggleModalForm(true)
}
</script>

<template>
	<div class="w-full">
		<div class="px-4 md:px-6">
			<div class="flex items-center justify-between gap-2 min-w-0">
				<div
					class="flex items-center gap-2 overflow-x-auto whitespace-nowrap scrollbar-none min-w-0 flex-1 py-1"
				>
					<template v-for="menu in menuList" :key="menu.key">
						<RouterLink
							:to="menu.to"
							class="h-9 px-4 inline-flex items-center justify-center rounded-[10px] whitespace-nowrap flex-shrink-0 text-sm border transition-all duration-150 ease-in-out select-none"
							:class="{
								'bg-card border-primary text-primary font-semibold shadow-xs':
									activePath === menu.key,
								'bg-card border-border text-muted-foreground font-medium hover:border-slate-400 dark:hover:border-slate-500 hover:text-foreground shadow-xs':
									activePath !== menu.key
							}"
						>
							{{ menu.label }}
						</RouterLink>
					</template>
				</div>

				<div class="flex-shrink-0 pl-1">
					<UiDropdownMenu>
						<UiDropdownMenuTrigger as-child>
							<UiButton
								variant="outline"
								size="icon"
								class="!h-9 !w-9 !rounded-[10px] !border-border !bg-card !text-muted-foreground hover:!border-slate-400 hover:!text-foreground shadow-xs"
							>
								<UiIcon name="menu" size="18px" />
							</UiButton>
						</UiDropdownMenuTrigger>

						<UiDropdownMenuContent align="end" class="w-48 p-1">
							<UiDropdownMenuItem class="cursor-pointer" @click="onClusterClick">
								<span>Cluster Monitoring</span>
							</UiDropdownMenuItem>
						</UiDropdownMenuContent>
					</UiDropdownMenu>
				</div>
			</div>
		</div>

		<ModalTable />
	</div>
</template>

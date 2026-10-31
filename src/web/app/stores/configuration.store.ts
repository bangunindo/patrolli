// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { ref } from 'vue'
import { defineStore } from 'pinia'

export const useConfigurationStore = defineStore(
	'configuration',
	() => {
		const drawerShow = ref<boolean>(false)
		const drawerTitle = ref<string>('Item Statistics')
		const toggleDrawer = (value?: boolean) => {
			if (typeof value === 'boolean') {
				drawerShow.value = value
			} else {
				drawerShow.value = !drawerShow.value
			}
		}

		return {
			drawerShow,
			drawerTitle,
			toggleDrawer
		}
	},
	{ persist: false }
)

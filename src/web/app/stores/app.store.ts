// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import appHelper from '@/app/helpers/app.helper'
import storageHelper from '@/app/helpers/storage.helper'
import { defineStore } from 'pinia'
import { ref, watch } from 'vue'

export const useAppStore = defineStore(
	'app',
	() => {
		const isDarkMode = ref(false)
		const setDarkModeStatus = (state: boolean) => {
			isDarkMode.value = state
		}

		const notificationItems = ref<any>([])

		const showFormModal = ref(false)
		const updateFormModal = (value: boolean) => {
			showFormModal.value = value
		}

		const showConfirmModal = ref(false)
		const updateConfirmModal = (value: boolean) => {
			showConfirmModal.value = value
		}

		const showError = (title = 'Error', message = '') => {
			notificationItems.value.push({
				title: title,
				description: message,
				type: 'error',
				variant: 'outline',
				dateAdded: new Date()
			})
		}

		const showSuccess = (title = 'Success', message = '') => {
			notificationItems.value.push({
				title: title,
				description: message,
				type: 'success',
				variant: 'outline',
				dateAdded: new Date()
			})
		}

		const getInitialActiveCluster = (): string | null => {
			if (typeof window !== 'undefined') {
				return storageHelper.loadItemStr('active_cluster_url')
			}
			return null
		}

		const activeClusterURL = ref<string | null>(getInitialActiveCluster())

		watch(
			activeClusterURL,
			(val) => {
				if (typeof window !== 'undefined') {
					if (val) {
						storageHelper.saveItemStr('active_cluster_url', val)
					} else {
						storageHelper.removeItem('active_cluster_url')
					}
				}
			},
			{ immediate: true }
		)

		const availableClusters = ref<any[]>(appHelper.parseClustersFromSource())
		const setAvailableClusters = (clusters: any[]) => {
			availableClusters.value = clusters
		}

		return {
			isDarkMode,
			setDarkModeStatus,
			notificationItems,
			showFormModal,
			updateFormModal,
			showConfirmModal,
			updateConfirmModal,
			showError,
			showSuccess,
			activeClusterURL,
			availableClusters,
			setAvailableClusters
		}
	},
	{ persist: true }
)

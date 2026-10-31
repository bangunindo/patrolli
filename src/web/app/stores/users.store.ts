// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { computed, reactive, ref } from 'vue'
import { defineStore } from 'pinia'
import { FormMode } from '@@/shared/types/app.type'
import type {
	CreateUserRequest,
	IClusterRoleAssignment,
	IListUsersQueryParams,
	IUserRecord,
	IUserStats,
	IUserSummaryData,
	UpdateUserRequest
} from '@@/shared/types/user.type'
import usersApi from '@/app/api/users.api'

export const useUsersStore = defineStore('users', () => {
	const users = ref<IUserRecord[]>([])
	const currentUser = ref<IUserRecord | null>(null)
	const userSummary = ref<IUserSummaryData | null>(null)
	const userStats = userSummary
	const userFormMode = ref<FormMode>(FormMode.ADD)

	const modalFormShow = ref(false)
	const modalDeleteShow = ref(false)
	const modalDetailShow = ref(false)
	const modalDeleteActionType = ref<'deactivate' | 'activate' | 'delete'>('deactivate')

	const loading = reactive({
		table: false,
		form: false
	})
	const error = ref<string | null>(null)

	const pagination = reactive({
		rows: 10,
		currentPage: 1,
		totalRecords: 0
	})

	const filter = reactive<IListUsersQueryParams>({
		username: undefined,
		limit: pagination.rows,
		offset: 0,
		order_by: undefined,
		order_dir: undefined
	})

	const filteredUsers = computed(() => users.value)

	const stats = computed<IUserStats>(() => {
		if (userSummary.value) {
			return {
				totalAllowSso: userSummary.value.total_allow_sso,
				totalDisabled: userSummary.value.total_disabled_user,
				totalActive: userSummary.value.total_active_user,
				totalLogin: userSummary.value.total_user,
				usersPerCluster: userSummary.value.user_cluster || {}
			}
		}

		let totalAllowSso = 0
		let totalDisabled = 0
		let totalActive = 0
		let totalLogin = 0

		users.value.forEach((user) => {
			if (user.allow_sso) {
				totalAllowSso++
			}
			if (user.adm_status === 0) {
				totalDisabled++
			} else if (user.adm_status === 1) {
				totalActive++
			}
			totalLogin++
		})

		return {
			totalAllowSso,
			totalDisabled,
			totalActive,
			totalLogin,
			usersPerCluster: {
				'All Registered': pagination.totalRecords
			}
		}
	})

	const toggleModalForm = (show: boolean, user?: IUserRecord) => {
		modalFormShow.value = show
		if (show && user) {
			userFormMode.value = FormMode.UPDATE
			currentUser.value = JSON.parse(JSON.stringify(user))
		} else if (show) {
			userFormMode.value = FormMode.ADD
			currentUser.value = null
		} else {
			currentUser.value = null
		}
	}

	const toggleModalDetail = (show: boolean, user?: IUserRecord | null) => {
		modalDetailShow.value = show
		if (user !== undefined) {
			currentUser.value = user ? JSON.parse(JSON.stringify(user)) : null
		}
	}

	const toggleModalDelete = (
		show: boolean,
		user?: IUserRecord | null,
		actionType?: 'deactivate' | 'activate' | 'delete'
	) => {
		modalDeleteShow.value = show
		if (user !== undefined) {
			currentUser.value = user ? JSON.parse(JSON.stringify(user)) : null
		}
		if (actionType) {
			modalDeleteActionType.value = actionType
		} else if (currentUser.value) {
			modalDeleteActionType.value =
				currentUser.value.adm_status === 1 ? 'deactivate' : 'delete'
		}
	}

	const fetchUserSummary = async (clusterURLs?: string[]) => {
		try {
			const response = await usersApi.getUserSummary(clusterURLs)
			if (response?.success && response.data) {
				userSummary.value = response.data
			}
			return response
		} catch (err: any) {
			console.error('Failed to fetch user summary:', err)
		}
	}

	const fetchUserStats = fetchUserSummary

	const fetchUsers = async (clusterURLs?: string[]) => {
		loading.table = true
		error.value = null
		try {
			filter.limit = pagination.rows
			filter.offset = (pagination.currentPage - 1) * pagination.rows

			const [response] = await Promise.all([
				usersApi.getUsers(filter, clusterURLs),
				fetchUserSummary(clusterURLs)
			])
			if (response?.success) {
				users.value = (response.data?.users ?? []).map((u: any) => ({
					...u,
					clusters: u.clusters || (u.cluster_name ? [u.cluster_name] : []),
					last_login_formatted: u.last_updated
						? new Date(u.last_updated).toLocaleString()
						: 'Never'
				}))
				pagination.totalRecords = response.data?.total ?? 0
			}
			return response
		} catch (err: any) {
			error.value = err.message || 'Failed to fetch users'
			throw err
		} finally {
			loading.table = false
		}
	}

	const createUser = async (
		payload: CreateUserRequest,
		clusterRoles?: IClusterRoleAssignment[],
		clusterURLs?: string[]
	) => {
		loading.form = true
		error.value = null
		try {
			const response = await usersApi.createUser(payload, clusterURLs)
			if (response?.success) {
				await fetchUsers(clusterURLs)
				return response.data
			}
			return response
		} catch (err: any) {
			error.value = err.message || 'Failed to create user'
			throw err
		} finally {
			loading.form = false
		}
	}

	const updateUser = async (
		userId: string,
		payload: UpdateUserRequest,
		clusterRoles?: IClusterRoleAssignment[],
		clusterURLs?: string[]
	) => {
		loading.form = true
		error.value = null
		try {
			const response = await usersApi.updateUser(userId, payload, clusterURLs)
			if (response?.success) {
				await fetchUsers(clusterURLs)
				return response.data
			}
			return response
		} catch (err: any) {
			error.value = err.message || 'Failed to update user'
			throw err
		} finally {
			loading.form = false
		}
	}

	const toggleUserStatus = async (
		userId: string,
		newStatus: number,
		clusterURLs?: string[],
		username?: string
	) => {
		loading.form = true
		error.value = null
		try {
			const uname = username || currentUser.value?.username
			const payload: UpdateUserRequest = uname
				? { adm_status: newStatus, username: uname }
				: { adm_status: newStatus }
			const response = uname
				? await usersApi.updateUser(userId, payload, clusterURLs, { username: uname })
				: await usersApi.updateUser(userId, payload, clusterURLs)
			if (response?.success) {
				await fetchUsers(clusterURLs)
				return response.data
			}
			return response
		} catch (err: any) {
			error.value = err.message || 'Failed to update user status'
			throw err
		} finally {
			loading.form = false
		}
	}

	const deleteUser = async (userId: string, clusterURLs?: string[], username?: string) => {
		loading.form = true
		error.value = null
		try {
			const uname = username || currentUser.value?.username
			const response = uname
				? await usersApi.deleteUser(userId, clusterURLs, uname)
				: await usersApi.deleteUser(userId, clusterURLs)
			if (response?.success) {
				await fetchUsers(clusterURLs)
				return response.data
			}
			return response
		} catch (err: any) {
			error.value = err.message || 'Failed to delete user'
			throw err
		} finally {
			loading.form = false
		}
	}

	const unlinkSso = async (userId: string, clusterURLs?: string[], username?: string) => {
		loading.form = true
		error.value = null
		try {
			const uname = username || currentUser.value?.username
			const response = uname
				? await usersApi.unlinkSso(userId, clusterURLs, uname)
				: await usersApi.unlinkSso(userId, clusterURLs)
			return response?.data
		} catch (err: any) {
			error.value = err.message || 'Failed to unlink SSO'
			throw err
		} finally {
			loading.form = false
		}
	}

	const resetFilter = () => {
		filter.username = undefined
		filter.offset = 0
		filter.order_by = undefined
		filter.order_dir = undefined
		pagination.currentPage = 1
	}

	return {
		users,
		currentUser,
		userFormMode,
		modalFormShow,
		modalDeleteShow,
		modalDetailShow,
		modalDeleteActionType,
		loading,
		error,
		pagination,
		filter,
		filteredUsers,
		stats,
		userSummary,
		userStats,
		fetchUserSummary,
		fetchUserStats,
		fetchUsers,
		toggleModalForm,
		toggleModalDelete,
		toggleModalDetail,
		createUser,
		updateUser,
		deleteUser,
		toggleUserStatus,
		unlinkSso,
		resetFilter
	}
})

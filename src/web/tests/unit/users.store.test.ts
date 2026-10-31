// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import { useUsersStore } from '@/stores/users.store'
import usersApi from '@/app/api/users.api'
import type { CreateUserRequest, UpdateUserRequest } from '@@/shared/types/user.type'

vi.mock('@/app/api/users.api', () => ({
	default: {
		getUsers: vi.fn(),
		getUser: vi.fn(),
		createUser: vi.fn(),
		updateUser: vi.fn(),
		deleteUser: vi.fn(),
		unlinkSso: vi.fn(),
		getUserSummary: vi.fn(),
		getUserStats: vi.fn()
	}
}))

describe('User Management - users.store API Integration', () => {
	const mockApiUsers = [
		{
			user_id: '11111111-1111-1111-1111-111111111111',
			username: 'biancavance',
			display_name: 'Bianca Vance',
			email: 'bianca.vance@patrolli.local',
			adm_status: 1,
			allow_sso: false,
			sso_linked: false,
			last_updated: '2026-10-08T00:00:00Z'
		},
		{
			user_id: '22222222-2222-2222-2222-222222222222',
			username: 'alyaputri',
			display_name: 'Alya Putri',
			email: 'alya.putri@patrolli.local',
			adm_status: 1,
			allow_sso: true,
			sso_linked: true,
			last_updated: '2026-10-07T00:00:00Z'
		},
		{
			user_id: '33333333-3333-3333-3333-333333333333',
			username: 'julian',
			display_name: 'Julian',
			email: 'julian@patrolli.local',
			adm_status: 0,
			allow_sso: false,
			sso_linked: false,
			last_updated: '2026-10-06T00:00:00Z'
		}
	]

	beforeEach(() => {
		setActivePinia(createPinia())
		vi.clearAllMocks()
	})

	it('should fetch users via usersApi.getUsers and update state', async () => {
		vi.mocked(usersApi.getUsers).mockResolvedValueOnce({
			success: true,
			message: 'OK',
			data: {
				total: 3,
				users: [...mockApiUsers]
			}
		} as any)

		const store = useUsersStore()
		expect(store.users.length).toBe(0)

		await store.fetchUsers()

		expect(usersApi.getUsers).toHaveBeenCalledTimes(1)
		expect(store.users.length).toBe(3)
		expect(store.pagination.totalRecords).toBe(3)
		expect(store.users[0].username).toBe('biancavance')
		expect(store.stats.totalActive).toBe(2)
		expect(store.stats.totalLogin).toBe(3)
	})

	it('should handle error when fetchUsers fails (Strict Live API mode)', async () => {
		vi.mocked(usersApi.getUsers).mockRejectedValueOnce(new Error('Network error 500'))

		const store = useUsersStore()
		await expect(store.fetchUsers()).rejects.toThrow('Network error 500')

		expect(store.error).toBe('Network error 500')
		expect(store.loading.table).toBe(false)
	})

	it('should create user via usersApi.createUser with clusterURLs and refresh users list', async () => {
		vi.mocked(usersApi.createUser).mockResolvedValueOnce({
			success: true,
			message: 'Created',
			data: {
				user_id: '44444444-4444-4444-4444-444444444444',
				username: 'alexturner',
				display_name: 'Alex Turner',
				email: 'alex@patrolli.local',
				adm_status: 1,
				allow_sso: false,
				sso_linked: false,
				last_updated: '2026-10-08T12:00:00Z'
			}
		} as any)

		vi.mocked(usersApi.getUsers).mockResolvedValueOnce({
			success: true,
			message: 'OK',
			data: {
				total: 4,
				users: [
					...mockApiUsers,
					{
						user_id: '44444444-4444-4444-4444-444444444444',
						username: 'alexturner',
						display_name: 'Alex Turner',
						email: 'alex@patrolli.local',
						adm_status: 1,
						allow_sso: false,
						sso_linked: false,
						last_updated: '2026-10-08T12:00:00Z'
					}
				]
			}
		} as any)

		const store = useUsersStore()
		const payload: CreateUserRequest = {
			username: 'alexturner',
			display_name: 'Alex Turner',
			email: 'alex@patrolli.local',
			password: 'securepassword123',
			adm_status: 1
		}
		const clusters = ['cluster-01', 'cluster-02']

		await store.createUser(payload, undefined, clusters)

		expect(usersApi.createUser).toHaveBeenCalledWith(payload, clusters)
		expect(usersApi.getUsers).toHaveBeenCalledTimes(1)
		expect(store.users.length).toBe(4)
	})

	it('should update user via usersApi.updateUser with clusterURLs and refresh users list', async () => {
		vi.mocked(usersApi.updateUser).mockResolvedValueOnce({
			success: true,
			message: 'Updated',
			data: {
				user_id: '11111111-1111-1111-1111-111111111111',
				username: 'biancavance',
				display_name: 'Bianca Vance Updated',
				email: 'bianca.vance@patrolli.local',
				adm_status: 1,
				allow_sso: false,
				sso_linked: false,
				last_updated: '2026-10-08T12:30:00Z'
			}
		} as any)

		vi.mocked(usersApi.getUsers).mockResolvedValueOnce({
			success: true,
			message: 'OK',
			data: {
				total: 3,
				users: [
					{
						...mockApiUsers[0],
						display_name: 'Bianca Vance Updated'
					},
					mockApiUsers[1],
					mockApiUsers[2]
				]
			}
		} as any)

		const store = useUsersStore()
		const payload: UpdateUserRequest = {
			adm_status: 1,
			display_name: 'Bianca Vance Updated'
		}
		const clusters = ['cluster-01']

		await store.updateUser('11111111-1111-1111-1111-111111111111', payload, undefined, clusters)

		expect(usersApi.updateUser).toHaveBeenCalledWith(
			'11111111-1111-1111-1111-111111111111',
			payload,
			clusters
		)
		expect(usersApi.getUsers).toHaveBeenCalledTimes(1)
		expect(store.users[0].display_name).toBe('Bianca Vance Updated')
	})

	it('should call usersApi.unlinkSso when unlinkSso is dispatched', async () => {
		vi.mocked(usersApi.unlinkSso).mockResolvedValueOnce({
			success: true,
			message: 'SSO Unlinked',
			data: {
				user_id: '22222222-2222-2222-2222-222222222222',
				username: 'alyaputri',
				display_name: 'Alya Putri',
				email: 'alya.putri@patrolli.local',
				adm_status: 1,
				allow_sso: false,
				sso_linked: false,
				last_updated: '2026-10-08T12:35:00Z'
			}
		} as any)

		const store = useUsersStore()
		const result = await store.unlinkSso(
			'22222222-2222-2222-2222-222222222222',
			['cluster-01'],
			'alyaputri'
		)

		expect(usersApi.unlinkSso).toHaveBeenCalledWith(
			'22222222-2222-2222-2222-222222222222',
			['cluster-01'],
			'alyaputri'
		)
		expect(result?.user_id).toBe('22222222-2222-2222-2222-222222222222')
	})

	it('should toggle user status via usersApi.updateUser', async () => {
		vi.mocked(usersApi.updateUser).mockResolvedValueOnce({
			success: true,
			message: 'Updated',
			data: {
				user_id: '11111111-1111-1111-1111-111111111111',
				username: 'biancavance',
				display_name: 'Bianca Vance',
				email: 'bianca.vance@patrolli.local',
				adm_status: 0,
				allow_sso: false,
				sso_linked: false,
				last_updated: '2026-10-08T12:45:00Z'
			}
		} as any)

		vi.mocked(usersApi.getUsers).mockResolvedValueOnce({
			success: true,
			message: 'OK',
			data: {
				total: 3,
				users: [{ ...mockApiUsers[0], adm_status: 0 }, mockApiUsers[1], mockApiUsers[2]]
			}
		} as any)

		const store = useUsersStore()
		await store.toggleUserStatus('11111111-1111-1111-1111-111111111111', 0)

		expect(usersApi.updateUser).toHaveBeenCalledWith(
			'11111111-1111-1111-1111-111111111111',
			{ adm_status: 0 },
			undefined
		)
		expect(store.users[0].adm_status).toBe(0)
	})

	it('should delete user via usersApi.deleteUser and refresh users list', async () => {
		vi.mocked(usersApi.deleteUser).mockResolvedValueOnce({
			success: true,
			message: 'Deleted',
			data: []
		} as any)

		vi.mocked(usersApi.getUsers).mockResolvedValueOnce({
			success: true,
			message: 'OK',
			data: {
				total: 2,
				users: [mockApiUsers[1], mockApiUsers[2]]
			}
		} as any)

		const store = useUsersStore()
		await store.deleteUser('11111111-1111-1111-1111-111111111111')

		expect(usersApi.deleteUser).toHaveBeenCalledWith(
			'11111111-1111-1111-1111-111111111111',
			undefined
		)
		expect(usersApi.getUsers).toHaveBeenCalledTimes(1)
		expect(store.users.length).toBe(2)
	})

	it('should validate CreateUserFormSchema and UpdateUserFormSchema correctly', async () => {
		const { CreateUserFormSchema, UpdateUserFormSchema } =
			await import('@@/shared/types/user.type')

		const missingPasswordResult = CreateUserFormSchema.safeParse({
			username: 'newuser',
			display_name: 'New User',
			email: 'newuser@patrolli.local',
			adm_status: 1
		})
		expect(missingPasswordResult.success).toBe(false)

		const emptyPasswordResult = CreateUserFormSchema.safeParse({
			username: 'newuser',
			display_name: 'New User',
			password: '',
			adm_status: 1
		})
		expect(emptyPasswordResult.success).toBe(false)

		const validCreateResult = CreateUserFormSchema.safeParse({
			username: 'newuser',
			display_name: 'New User',
			password: 'valid_secure_password',
			adm_status: 1
		})
		expect(validCreateResult.success).toBe(true)
		expect((validCreateResult as any).data.allow_sso).toBe(true)

		const validUpdateEmptyPassword = UpdateUserFormSchema.safeParse({
			adm_status: 1,
			display_name: 'Updated Name',
			password: ''
		})
		expect(validUpdateEmptyPassword.success).toBe(true)

		const validUpdateNoPassword = UpdateUserFormSchema.safeParse({
			adm_status: 1,
			display_name: 'Updated Name'
		})
		expect(validUpdateNoPassword.success).toBe(true)
	}, 30000)

	it('should fetch user summary and update store stats computation with allow_sso and disabled counts', async () => {
		const mockSummary = {
			status_code: 200,
			success: true,
			data: {
				total_user: 12,
				total_active_user: 10,
				total_disabled_user: 2,
				total_allow_sso: 9,
				user_cluster: {
					'cluster-alpha': 8,
					'cluster-beta': 4
				}
			}
		}
		vi.mocked(usersApi.getUserSummary).mockResolvedValueOnce(mockSummary as any)

		const store = useUsersStore()
		await store.fetchUserSummary()

		expect(usersApi.getUserSummary).toHaveBeenCalledTimes(1)
		expect(store.userSummary).toEqual(mockSummary.data)
		expect(store.stats.totalAllowSso).toBe(9)
		expect(store.stats.totalDisabled).toBe(2)
		expect(store.stats.totalActive).toBe(10)
		expect(store.stats.totalLogin).toBe(12)
		expect(store.stats.usersPerCluster).toEqual({
			'cluster-alpha': 8,
			'cluster-beta': 4
		})
	})

	it('should toggle user status with username and clusterURLs via usersApi.updateUser', async () => {
		vi.mocked(usersApi.updateUser).mockResolvedValueOnce({
			status_code: 200,
			success: true,
			data: { adm_status: 0 }
		} as any)
		vi.mocked(usersApi.getUsers).mockResolvedValueOnce({
			success: true,
			data: { total: 0, users: [] }
		} as any)

		const store = useUsersStore()
		store.currentUser = {
			user_id: 'user-123',
			username: 'alice',
			adm_status: 1
		} as any

		await store.toggleUserStatus('user-123', 0, ['cluster-01', 'cluster-02'])

		expect(usersApi.updateUser).toHaveBeenCalledWith(
			'user-123',
			{ adm_status: 0, username: 'alice' },
			['cluster-01', 'cluster-02'],
			{ username: 'alice' }
		)
	})
})

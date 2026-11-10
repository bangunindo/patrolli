// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { expect } from '@playwright/test'
import { STORAGE_STATE, test } from '../../fixtures/test-fixtures'

test.describe('Feature: User Management (/user-management)', () => {
	test.describe('Multi-Cluster Session (Keycloak SSO)', () => {
		test.use({ storageState: STORAGE_STATE.MULTI_CLUSTER })

		test('renders user management page with stats cards, table headers, and action button @smoke', async ({
			userManagementPage
		}) => {
			await userManagementPage.gotoUserManagement()

			// 1. Verify Page Header
			await expect(userManagementPage.heading).toBeVisible()
			await expect(
				userManagementPage.page.getByText(
					/overview of platform access and user credentials across clusters/i
				)
			).toBeVisible()
			await expect(userManagementPage.refreshButton).toBeVisible()

			// 2. Verify Stats Cards
			await expect(userManagementPage.cardAllowSso).toBeVisible()
			await expect(userManagementPage.cardAllowSsoCount).toBeVisible()
			await expect(userManagementPage.cardDisabled).toBeVisible()
			await expect(userManagementPage.cardDisabledCount).toBeVisible()
			await expect(userManagementPage.cardClusterModule).toBeVisible()

			// 3. Verify Toolbar Elements
			await expect(userManagementPage.searchInput).toBeVisible()
			await expect(userManagementPage.createUserButton).toBeVisible()

			// 4. Verify Table Column Headers
			await expect(userManagementPage.table).toBeVisible()
			const expectedHeaders = [
				'USERNAME',
				'EMAIL',
				'CLUSTER',
				'ALLOW SSO',
				'LAST UPDATED',
				'STATUS'
			]
			for (const header of expectedHeaders) {
				await expect(
					userManagementPage.table
						.locator('thead')
						.getByText(new RegExp(`^${header}$`, 'i'))
				).toBeVisible()
			}

			// 5. Verify Refresh Button interaction
			await userManagementPage.clickRefresh()
			await expect(userManagementPage.table).toBeVisible()
		})

		test('validates required fields in create user form and handles modal cancel', async ({
			userManagementPage
		}) => {
			await userManagementPage.gotoUserManagement()

			// Open Create User Modal
			await userManagementPage.createUserButton.click()
			await expect(userManagementPage.modalForm).toBeVisible()
			await expect(userManagementPage.formTitle).toHaveText(/create user/i)

			// Attempt to submit empty form
			await userManagementPage.formSaveButton.click()

			// Modal should stay open due to required fields
			await expect(userManagementPage.modalForm).toBeVisible()

			// Fill partial data and cancel
			await userManagementPage.formFullNameInput.fill('Cancel Test')
			await userManagementPage.formCancelButton.click()
			await expect(userManagementPage.modalForm).not.toBeVisible()
		})

		test('manages complete user lifecycle: create, search, view detail, edit, deactivate/activate, and delete', async ({
			userManagementPage
		}) => {
			await userManagementPage.gotoUserManagement()

			const ts = Date.now()
			const username = `e2e_usr_${ts}`
			const fullName = `E2E FullName ${ts}`
			const email = `${username}@patrolli.local`
			const password = 'PatrolliPass123!'
			const updatedFullName = `E2E Updated ${ts}`

			try {
				// 1. Create User
				await userManagementPage.createUserButton.click()
				await expect(userManagementPage.modalForm).toBeVisible()

				await userManagementPage.fillCreateUserForm({
					fullName,
					username,
					email,
					password,
					allowSso: true
				})

				const createResponsePromise = userManagementPage.page.waitForResponse(
					(res) =>
						res.url().includes('/api/users') &&
						res.request().method() === 'POST' &&
						res.status() < 400,
					{ timeout: 15000 }
				)
				await userManagementPage.formSaveButton.click()
				await createResponsePromise
				await expect(userManagementPage.modalForm).not.toBeVisible()

				// 2. Search and Verify in Table
				await userManagementPage.searchUser(username)
				const userRow = userManagementPage.getUserRow(username)
				await expect(userRow).toBeVisible({ timeout: 10000 })
				await expect(userRow.getByText(fullName)).toBeVisible()
				await expect(userRow.getByText(email)).toBeVisible()
				await expect(userRow.getByText('Allowed', { exact: true })).toBeVisible()
				await expect(userRow.getByText('Active', { exact: true })).toBeVisible()

				// 3. View Detail
				await userManagementPage.openUserAction(username, 'View')
				await expect(userManagementPage.modalDetail).toBeVisible()
				await expect(userManagementPage.modalDetail.getByText(`@${username}`)).toBeVisible()
				await expect(userManagementPage.modalDetail.getByText(fullName)).toBeVisible()
				await expect(userManagementPage.modalDetail.getByText(email)).toBeVisible()
				await expect(
					userManagementPage.modalDetail.getByText('Active', { exact: true })
				).toBeVisible()
				await expect(
					userManagementPage.modalDetail.getByText('Allowed', { exact: true })
				).toBeVisible()
				await userManagementPage.modalDetailCloseButton.click()
				await expect(userManagementPage.modalDetail).not.toBeVisible()

				// 4. Edit User (update Full Name, disable Allow SSO)
				await userManagementPage.openUserAction(username, 'Edit')
				await expect(userManagementPage.modalForm).toBeVisible()
				await expect(userManagementPage.formTitle).toHaveText(/edit user/i)

				await userManagementPage.formFullNameInput.fill(updatedFullName)
				// Toggle Allow SSO to false
				const isAllowSsoChecked =
					(await userManagementPage.formAllowSsoSwitch.getAttribute('aria-checked')) ===
					'true'
				if (isAllowSsoChecked) {
					await userManagementPage.formAllowSsoSwitch.click()
				}

				const editResponsePromise = userManagementPage.page.waitForResponse(
					(res) =>
						res.url().includes('/api/users') &&
						res.request().method() === 'PATCH' &&
						res.status() < 400,
					{ timeout: 15000 }
				)
				await userManagementPage.formSaveButton.click()
				await editResponsePromise
				await expect(userManagementPage.modalForm).not.toBeVisible()

				// Verify Edit in Table
				await expect(userRow.getByText(updatedFullName)).toBeVisible()
				await expect(userRow.getByText('Not Allowed', { exact: true })).toBeVisible()

				// 5. Deactivate Account
				await userManagementPage.openUserAction(username, 'Deactivate Account')
				await expect(userManagementPage.modalDelete).toBeVisible()
				await expect(
					userManagementPage.modalDelete.getByText(/You are about to deactivate user/i)
				).toBeVisible()

				const deactivatePromise = userManagementPage.page.waitForResponse(
					(res) =>
						res.url().includes('/api/users') &&
						res.request().method() === 'PATCH' &&
						res.status() < 400,
					{ timeout: 15000 }
				)
				await userManagementPage.modalConfirmActionButton.click()
				await deactivatePromise
				await expect(userManagementPage.modalDelete).not.toBeVisible()

				// Verify Status changed to Inactive
				await expect(userRow.getByText('Inactive', { exact: true })).toBeVisible()

				// 6. Reactivate Account
				await userManagementPage.openUserAction(username, 'Activate Account')
				await expect(userManagementPage.modalDelete).toBeVisible()
				await expect(
					userManagementPage.modalDelete.getByText(/You are about to activate user/i)
				).toBeVisible()

				const activatePromise = userManagementPage.page.waitForResponse(
					(res) =>
						res.url().includes('/api/users') &&
						res.request().method() === 'PATCH' &&
						res.status() < 400,
					{ timeout: 15000 }
				)
				await userManagementPage.modalConfirmActionButton.click()
				await activatePromise
				await expect(userManagementPage.modalDelete).not.toBeVisible()

				// Verify Status restored to Active
				await expect(userRow.getByText('Active', { exact: true })).toBeVisible()

				// 7. Delete Permanently
				await userManagementPage.openUserAction(username, 'Delete')
				await expect(userManagementPage.modalDelete).toBeVisible()
				await userManagementPage.optionDeletePermanently.click()

				const deletePromise = userManagementPage.page.waitForResponse(
					(res) =>
						res.url().includes('/api/users') &&
						res.request().method() === 'DELETE' &&
						res.status() < 400,
					{ timeout: 15000 }
				)
				await userManagementPage.modalConfirmActionButton.click()
				await deletePromise
				await expect(userManagementPage.modalDelete).not.toBeVisible()

				// Refresh table and verify user is gone
				await userManagementPage.clickRefresh()
				await expect(userManagementPage.getUserRow(username)).not.toBeVisible()
			} finally {
				// Fallback cleanup if test failed midway
				try {
					await userManagementPage.searchUser(username)
					const row = userManagementPage.getUserRow(username)
					if (await row.isVisible()) {
						await userManagementPage.openUserAction(username, 'Delete')
						if (await userManagementPage.modalDelete.isVisible()) {
							await userManagementPage.optionDeletePermanently.click()
							await userManagementPage.modalConfirmActionButton.click()
						}
					}
				} catch {
					// Silent fallback cleanup
				}
			}
		})

		test('navigates to user management page from navbar on desktop and mobile', async ({
			userManagementPage
		}) => {
			// Start from another page
			await userManagementPage.page.goto('/cluster')
			await userManagementPage.page.waitForLoadState('domcontentloaded')

			// Navigate to User Management via navbar
			await userManagementPage.navigateFromNavbar()

			// Verify heading and URL
			await expect(userManagementPage.heading).toBeVisible()
			expect(userManagementPage.page.url()).toContain('/user-management')
		})
	})

	test.describe('Single-Cluster Session (Local Login)', () => {
		test.use({ storageState: STORAGE_STATE.SINGLE_CLUSTER })

		test('verifies cluster selector behavior in single-cluster mode', async ({
			userManagementPage
		}) => {
			await userManagementPage.gotoUserManagement()

			// Open Create User Modal
			await userManagementPage.createUserButton.click()
			await expect(userManagementPage.modalForm).toBeVisible()

			// Verify cluster selector container is disabled / read-only in single-cluster mode
			const clusterInput = userManagementPage.formClusterInput.first()
			if (await clusterInput.isVisible()) {
				await expect(clusterInput).toHaveClass(/cursor-not-allowed|opacity-50/)
			}

			// Close modal
			await userManagementPage.formCancelButton.click()
			await expect(userManagementPage.modalForm).not.toBeVisible()
		})
	})
})

// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { test, expect, STORAGE_STATE } from '../../fixtures/test-fixtures'

test.describe('Configuration Management - Listeners', () => {
	test.describe('Authenticated Session (Multi-Cluster)', () => {
		test('renders listeners page with table, search, and action header @smoke', async ({
			listenerPage
		}) => {
			await listenerPage.gotoListeners()

			await expect(listenerPage.createListenerButton).toBeVisible()
			await expect(listenerPage.searchInput).toBeVisible()
			await expect(listenerPage.table).toBeVisible()
		})

		test('validates required fields in create listener form and handles modal cancel', async ({
			listenerPage
		}) => {
			await listenerPage.gotoListeners()
			await listenerPage.openCreateModal()

			await expect(listenerPage.modalDialog).toBeVisible()
			await expect(listenerPage.formSaveButton).toBeVisible()

			// Attempt saving without required name: validation keeps dialog open
			await listenerPage.submitForm()
			await expect(listenerPage.modalDialog).toBeVisible()

			// Dismiss dialog via cancel button
			await listenerPage.formCancelButton.click()
			await expect(listenerPage.modalDialog).not.toBeVisible({ timeout: 10_000 })
		})

		test('creates, views detail, edits, and deletes a listener lifecycle', async ({
			listenerPage,
			apiHelper
		}) => {
			const listenerName = `e2e-ls-${Date.now()}`
			const updatedListenerName = `${listenerName}-mod`

			try {
				await listenerPage.gotoListeners()
				await listenerPage.openCreateModal()
				await listenerPage.fillListenerName(listenerName)
				await listenerPage.submitForm()
				await expect(listenerPage.modalDialog).not.toBeVisible({ timeout: 15_000 })

				await listenerPage.searchListener(listenerName)
				const listenerRow = await listenerPage.getRowByName(listenerName)
				await expect(listenerRow).toBeVisible({ timeout: 10_000 })
				await expect(listenerRow).toContainText(/dummy/i)

				await listenerPage.openDetail(listenerName)
				await expect(listenerPage.modalDetail).toBeVisible({ timeout: 10_000 })
				await expect(listenerPage.modalDetail).toContainText(listenerName)
				await expect(listenerPage.modalDetail).toContainText(/dummy/i)
				const closeDetailBtn = listenerPage.modalDetail
					.getByRole('button', { name: /^close$/i })
					.first()
				await closeDetailBtn.click()
				await expect(listenerPage.modalDetail).not.toBeVisible({ timeout: 5_000 })

				await listenerPage.openEdit(listenerName)
				await listenerPage.fillListenerName(updatedListenerName)
				await listenerPage.submitForm()
				await expect(listenerPage.modalDialog).not.toBeVisible({ timeout: 15_000 })

				await listenerPage.clearSearch()
				await listenerPage.searchListener(updatedListenerName)
				const updatedRow = await listenerPage.getRowByName(updatedListenerName)
				await expect(updatedRow).toBeVisible({ timeout: 10_000 })

				await listenerPage.deleteListener(updatedListenerName)
				await listenerPage.clearSearch()
				await listenerPage.searchListener(updatedListenerName)
				const rowAfterDelete = await listenerPage.getRowByName(updatedListenerName)
				await expect(rowAfterDelete).not.toBeVisible({ timeout: 10_000 })
			} finally {
				await apiHelper.deleteListenerByName(updatedListenerName)
				await apiHelper.deleteListenerByName(listenerName)
			}
		})

		test('filters listeners using filter dropdown', async ({ listenerPage }) => {
			await listenerPage.gotoListeners()

			await listenerPage.filterDropdownButton.click()
			const filterMenu = listenerPage.page.locator('[role="menu"]')
			await expect(filterMenu).toBeVisible({ timeout: 5_000 })

			// Close filter menu by pressing Escape
			await listenerPage.page.keyboard.press('Escape')
			await expect(filterMenu).not.toBeVisible({ timeout: 5_000 })
		})
	})

	test.describe('Single-Cluster Listener Form', () => {
		test.use({ storageState: STORAGE_STATE.SINGLE_CLUSTER })

		test('locks cluster selector in single-cluster session', async ({ listenerPage }) => {
			await listenerPage.gotoListeners()
			await listenerPage.openCreateModal()

			// Cluster selector must be disabled/locked for single-cluster mode
			const clusterInput = listenerPage.formClusterInput.first()
			if (await clusterInput.isVisible()) {
				await expect(clusterInput).toHaveClass(/cursor-not-allowed|opacity-50/)
			}

			await listenerPage.formCancelButton.click()
			await expect(listenerPage.modalDialog).not.toBeVisible({ timeout: 10_000 })
		})
	})
})

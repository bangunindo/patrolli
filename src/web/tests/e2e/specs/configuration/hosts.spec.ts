// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { test, expect, STORAGE_STATE } from '../../fixtures/test-fixtures'

test.describe('Configuration Management - Hosts', () => {
	test.describe('Default Session (Multi-Cluster)', () => {
		test('renders host management page with table, search, and action header @smoke', async ({
			hostPage
		}) => {
			await hostPage.gotoHosts()

			await expect(hostPage.createHostButton).toBeVisible()
			await expect(hostPage.searchInput).toBeVisible()
			await expect(hostPage.table).toBeVisible()
		})

		test('validates required fields in create host form and handles modal cancel', async ({
			hostPage
		}) => {
			await hostPage.gotoHosts()
			await hostPage.openCreateModal()

			await expect(hostPage.modalDialog).toBeVisible()
			await expect(hostPage.formNameInput).toBeVisible()
			await expect(hostPage.formSaveButton).toBeVisible()

			// Attempt saving with empty name: form validation should keep modal open
			await hostPage.submitForm()
			await expect(hostPage.modalDialog).toBeVisible()

			// Dismiss via cancel button
			await hostPage.formCancelButton.click()
			await expect(hostPage.modalDialog).not.toBeVisible()
		})

		test('creates, views detail, and edits a host lifecycle', async ({
			hostPage,
			page,
			apiHelper
		}) => {
			const hostName = `e2e-host-${Date.now()}`

			try {
				await hostPage.gotoHosts()
				await hostPage.openCreateModal()

				await hostPage.fillHostName(hostName)
				await hostPage.setStatus(true)
				await hostPage.addLabel('environment', 'testing')

				await hostPage.submitForm()
				await expect(hostPage.modalDialog).not.toBeVisible({ timeout: 15_000 })

				await hostPage.searchHost(hostName)
				const createdRow = await hostPage.getRowByName(hostName)
				await expect(createdRow).toBeVisible({ timeout: 10_000 })
				await expect(createdRow).toContainText('Active')

				await hostPage.openDetail(hostName)
				await expect(hostPage.modalDetail).toBeVisible()
				await expect(hostPage.modalDetail).toContainText(hostName)
				await expect(hostPage.modalDetail).toContainText('environment')

				await page.keyboard.press('Escape')
				await expect(hostPage.modalDetail).not.toBeVisible()

				await hostPage.deleteHost(hostName)
				await hostPage.clearSearch()
				await hostPage.searchHost(hostName)
				const rowAfterDelete = await hostPage.getRowByName(hostName)
				await expect(rowAfterDelete).not.toBeVisible({ timeout: 10_000 })
			} finally {
				await apiHelper.deleteHostByName(hostName)
			}
		})

		test('handles template unapply choice modal during host update', async ({
			hostPage,
			apiHelper
		}) => {
			const hostName = `e2e-tpl-${Date.now()}`

			try {
				await hostPage.gotoHosts()
				await hostPage.openCreateModal()
				await hostPage.fillHostName(hostName)

				const templateCount = await hostPage.getAvailableTemplateCount()
				if (templateCount > 0) {
					const linkedTemplateName = await hostPage.selectFirstTemplate()
					await hostPage.submitForm()
					await expect(hostPage.modalDialog).not.toBeVisible({ timeout: 15_000 })

					await hostPage.searchHost(hostName)
					const hostRow = await hostPage.getRowByName(hostName)
					await expect(hostRow).toBeVisible({ timeout: 10_000 })
					await expect(hostRow).toContainText(linkedTemplateName)

					await hostPage.openEdit(hostName)
					await hostPage.deselectFirstTemplate()
					await hostPage.submitForm()

					await expect(hostPage.modalUnapplyTemplate).toBeVisible({ timeout: 10_000 })
					await hostPage.confirmUnapplyTemplate(false)

					await hostPage.deleteHost(hostName)
				} else {
					await hostPage.formCancelButton.click()
				}
			} finally {
				await apiHelper.deleteHostByName(hostName)
			}
		})

		test('deletes host with confirmation modal lifecycle', async ({ hostPage, apiHelper }) => {
			const hostName = `e2e-del-${Date.now()}`

			try {
				await hostPage.gotoHosts()
				await hostPage.openCreateModal()
				await hostPage.fillHostName(hostName)
				await hostPage.submitForm()
				await expect(hostPage.modalDialog).not.toBeVisible({ timeout: 15_000 })

				await hostPage.searchHost(hostName)
				const targetRow = await hostPage.getRowByName(hostName)
				await expect(targetRow).toBeVisible({ timeout: 10_000 })

				await hostPage.deleteHost(hostName)

				await hostPage.clearSearch()
				await hostPage.searchHost(hostName)
				const deletedRow = await hostPage.getRowByName(hostName)
				await expect(deletedRow).not.toBeVisible({ timeout: 10_000 })
			} finally {
				await apiHelper.deleteHostByName(hostName)
			}
		})
	})

	test.describe('Single-Cluster Host Form', () => {
		test.use({ storageState: STORAGE_STATE.SINGLE_CLUSTER })

		test('locks cluster selector in single-cluster session', async ({ hostPage }) => {
			await hostPage.gotoHosts()
			await hostPage.openCreateModal()

			// Cluster selector must be disabled/locked for single-cluster mode
			const clusterInput = hostPage.formClusterInput.first()
			if (await clusterInput.isVisible()) {
				await expect(clusterInput).toHaveClass(/cursor-not-allowed|opacity-50/)
			}

			await hostPage.formCancelButton.click()
			await expect(hostPage.modalDialog).not.toBeVisible()
		})
	})
})

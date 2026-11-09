// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { test, expect, STORAGE_STATE } from '../../fixtures/test-fixtures'

test.describe('Configuration Management - Endpoints', () => {
	test.describe('Authenticated Session (Multi-Cluster)', () => {
		test('renders endpoints page with table, search, and action header @smoke', async ({
			endpointPage
		}) => {
			await endpointPage.gotoEndpoints()

			await expect(endpointPage.createEndpointButton).toBeVisible()
			await expect(endpointPage.searchInput).toBeVisible()
			await expect(endpointPage.table).toBeVisible()
		})

		test('validates required fields in create endpoint form and handles modal cancel', async ({
			endpointPage
		}) => {
			await endpointPage.gotoEndpoints()
			await endpointPage.openCreateModal()

			await expect(endpointPage.modalDialog).toBeVisible()
			await expect(endpointPage.formSaveButton).toBeVisible()

			// Attempt saving without required name & host: validation keeps dialog open
			await endpointPage.submitForm()
			await expect(endpointPage.modalDialog).toBeVisible()

			// Dismiss dialog via cancel button
			await endpointPage.formCancelButton.click()
			await expect(endpointPage.modalDialog).not.toBeVisible({ timeout: 10_000 })
		})

		test('validates HTTP protocol requirements in endpoint form', async ({ endpointPage }) => {
			await endpointPage.gotoEndpoints()
			await endpointPage.openCreateModal()
			await expect(endpointPage.modalDialog).toBeVisible()

			// Select HTTP protocol
			await endpointPage.selectProtocol('http')

			// HTTP URL input should be displayed
			await expect(endpointPage.formUrlInput).toBeVisible({ timeout: 5_000 })

			// Fill invalid url and submit
			await endpointPage.fillEndpointName(`e2e-http-${Date.now()}`)
			await endpointPage.fillHttpUrl('invalid-url-no-scheme')
			await endpointPage.submitForm()

			// Validation retains the dialog
			await expect(endpointPage.modalDialog).toBeVisible()

			// Cancel form dialog
			await endpointPage.formCancelButton.click()
			await expect(endpointPage.modalDialog).not.toBeVisible({ timeout: 10_000 })
		})

		test('creates, views detail, and edits an endpoint lifecycle with dynamic host', async ({
			hostPage,
			endpointPage,
			apiHelper
		}) => {
			const hostName = `e2e-h-ep-${Date.now()}`
			const endpointName = `e2e-ep-${Date.now()}`
			const updatedEndpointName = `${endpointName}-mod`

			try {
				await hostPage.gotoHosts()
				await hostPage.openCreateModal()
				await hostPage.fillHostName(hostName)
				await hostPage.submitForm()
				await expect(hostPage.modalDialog).not.toBeVisible({ timeout: 15_000 })

				await endpointPage.gotoEndpoints()
				await endpointPage.openCreateModal()
				await endpointPage.fillEndpointName(endpointName)
				await endpointPage.selectHost(hostName)
				await endpointPage.fillSnmpV1OrV2c('192.168.1.100:161', 'public')
				await endpointPage.submitForm()
				await expect(endpointPage.modalDialog).not.toBeVisible({ timeout: 15_000 })

				await endpointPage.searchEndpoint(endpointName)
				const endpointRow = await endpointPage.getRowByName(endpointName)
				await expect(endpointRow).toBeVisible({ timeout: 10_000 })
				await expect(endpointRow).toContainText(hostName)

				await endpointPage.openDetail(endpointName)
				await expect(endpointPage.modalDetail).toBeVisible({ timeout: 10_000 })
				await expect(endpointPage.modalDetail).toContainText(endpointName)
				await expect(endpointPage.modalDetail).toContainText(hostName)
				const closeDetailBtn = endpointPage.modalDetail
					.getByRole('button', { name: /^close$/i })
					.first()
				await closeDetailBtn.click()
				await expect(endpointPage.modalDetail).not.toBeVisible({ timeout: 5_000 })

				await endpointPage.openEdit(endpointName)
				await endpointPage.fillEndpointName(updatedEndpointName)
				await endpointPage.submitForm()
				await expect(endpointPage.modalDialog).not.toBeVisible({ timeout: 15_000 })

				await endpointPage.clearSearch()
				await endpointPage.searchEndpoint(updatedEndpointName)
				const updatedRow = await endpointPage.getRowByName(updatedEndpointName)
				await expect(updatedRow).toBeVisible({ timeout: 10_000 })

				await endpointPage.deleteEndpoint(updatedEndpointName)
				await endpointPage.clearSearch()
				await endpointPage.searchEndpoint(updatedEndpointName)
				const rowAfterDelete = await endpointPage.getRowByName(updatedEndpointName)
				await expect(rowAfterDelete).not.toBeVisible({ timeout: 10_000 })
			} finally {
				await apiHelper.deleteEndpointByName(updatedEndpointName)
				await apiHelper.deleteEndpointByName(endpointName)
				await apiHelper.deleteHostByName(hostName)
			}
		})

		test('filters endpoints using filter dropdown', async ({ endpointPage }) => {
			await endpointPage.gotoEndpoints()

			await endpointPage.filterDropdownButton.click()
			const filterMenu = endpointPage.page.locator('[role="menu"]')
			await expect(filterMenu).toBeVisible({ timeout: 5_000 })

			// Close filter menu by pressing Escape
			await endpointPage.page.keyboard.press('Escape')
			await expect(filterMenu).not.toBeVisible({ timeout: 5_000 })
		})
	})

	test.describe('Single-Cluster Endpoint Form', () => {
		test.use({ storageState: STORAGE_STATE.SINGLE_CLUSTER })

		test('locks cluster selector in single-cluster session', async ({ endpointPage }) => {
			await endpointPage.gotoEndpoints()
			await endpointPage.openCreateModal()

			// Cluster selector must be disabled/locked for single-cluster mode
			const clusterInput = endpointPage.formClusterInput.first()
			if (await clusterInput.isVisible()) {
				await expect(clusterInput).toHaveClass(/cursor-not-allowed|opacity-50/)
			}

			await endpointPage.formCancelButton.click()
			await expect(endpointPage.modalDialog).not.toBeVisible({ timeout: 10_000 })
		})
	})
})

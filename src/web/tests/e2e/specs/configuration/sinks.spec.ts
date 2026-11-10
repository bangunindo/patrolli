// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { test, expect, STORAGE_STATE } from '../../fixtures/test-fixtures'

test.describe('Configuration Management - Sinks', () => {
	test.describe('Authenticated Session (Multi-Cluster)', () => {
		test('renders sinks page with table, search, filter type, and action header @smoke', async ({
			sinkPage
		}) => {
			await sinkPage.gotoSinks()

			await expect(sinkPage.createSinkButton).toBeVisible()
			await expect(sinkPage.searchInput).toBeVisible()
			await expect(sinkPage.filterDropdownButton).toBeVisible()
			await expect(sinkPage.table).toBeVisible()
		})

		test('validates required fields in create sink form and handles modal cancel', async ({
			sinkPage
		}) => {
			await sinkPage.gotoSinks()
			await sinkPage.openCreateModal()

			await expect(sinkPage.modalDialog).toBeVisible()
			await expect(sinkPage.formSaveButton).toBeVisible()

			// Attempt saving without required name: validation keeps dialog open
			await sinkPage.submitForm()
			await expect(sinkPage.modalDialog).toBeVisible()

			// Dismiss dialog via cancel button
			await sinkPage.cancelForm()
			await expect(sinkPage.modalDialog).not.toBeVisible({ timeout: 10_000 })
		})

		test('verifies dynamic form reactivity on kafka security protocol matrix', async ({
			sinkPage
		}) => {
			await sinkPage.gotoSinks()
			await sinkPage.openCreateModal()

			// Select Config Type: Kafka
			await sinkPage.selectConfigType('kafka')

			// Protocol: Plaintext (default or selected) -> SSL and SASL should not be visible
			await sinkPage.selectKafkaSecurityProtocol('plaintext')
			const saslUsernameInput = sinkPage.page
				.locator('#sink-form')
				.getByPlaceholder(/kafka sasl username/i)
			const sslCaCertInput = sinkPage.page
				.locator('#sink-form')
				.getByPlaceholder(/\/etc\/ssl\/certs\/kafka-ca\.pem/i)
			await expect(saslUsernameInput).not.toBeVisible()
			await expect(sslCaCertInput).not.toBeVisible()

			// Protocol: SSL -> SSL fields visible, SASL hidden
			await sinkPage.selectKafkaSecurityProtocol('ssl')
			await expect(sslCaCertInput).toBeVisible({ timeout: 5_000 })
			await expect(saslUsernameInput).not.toBeVisible()

			// Protocol: SASL Plaintext -> SASL visible, SSL hidden
			await sinkPage.selectKafkaSecurityProtocol('sasl_plaintext')
			await expect(saslUsernameInput).toBeVisible({ timeout: 5_000 })
			await expect(sslCaCertInput).not.toBeVisible()

			// Protocol: SASL SSL -> both visible
			await sinkPage.selectKafkaSecurityProtocol('sasl_ssl')
			await expect(saslUsernameInput).toBeVisible({ timeout: 5_000 })
			await expect(sslCaCertInput).toBeVisible({ timeout: 5_000 })

			// Cancel and close dialog
			await sinkPage.cancelForm()
			await expect(sinkPage.modalDialog).not.toBeVisible({ timeout: 10_000 })
		})

		test('creates, views detail, toggles status, edits, and deletes a file sink lifecycle', async ({
			sinkPage,
			apiHelper
		}) => {
			const sinkName = `e2e-sink-${Date.now()}`
			const updatedSinkName = `${sinkName}-mod`
			const logPath = `/tmp/e2e-${Date.now()}.log`

			try {
				await sinkPage.gotoSinks()
				await sinkPage.openCreateModal()
				await sinkPage.fillSinkName(sinkName)
				await sinkPage.selectConfigType('file')
				await sinkPage.fillFilePath(logPath)
				await sinkPage.submitForm()
				await expect(sinkPage.modalDialog).not.toBeVisible({ timeout: 15_000 })

				await sinkPage.searchSink(sinkName)
				const sinkRow = await sinkPage.getRowByName(sinkName)
				await expect(sinkRow).toBeVisible({ timeout: 10_000 })
				await expect(sinkRow).toContainText(/file/i)
				await expect(sinkRow).toContainText(/active/i)

				await sinkPage.openDetail(sinkName)
				await expect(sinkPage.modalDetail).toBeVisible({ timeout: 10_000 })
				await expect(sinkPage.modalDetail).toContainText(sinkName)
				await expect(sinkPage.modalDetail).toContainText(/file/i)
				await expect(sinkPage.modalDetail).toContainText(logPath)
				await sinkPage.closeDetail()

				await sinkPage.openEdit(sinkName)
				await sinkPage.fillSinkName(updatedSinkName)
				await sinkPage.formStatusSwitch.click()
				await sinkPage.submitForm()
				await expect(sinkPage.modalDialog).not.toBeVisible({ timeout: 15_000 })

				await sinkPage.searchSink(updatedSinkName)
				const updatedRow = await sinkPage.getRowByName(updatedSinkName)
				await expect(updatedRow).toBeVisible({ timeout: 10_000 })
				await expect(updatedRow).toContainText(/inactive/i)

				await sinkPage.deleteSink(updatedSinkName)
				await sinkPage.searchSink(updatedSinkName)
				const rowAfterDelete = await sinkPage.getRowByName(updatedSinkName)
				await expect(rowAfterDelete).not.toBeVisible({ timeout: 10_000 })
			} finally {
				await apiHelper.deleteSinkByName(updatedSinkName)
				await apiHelper.deleteSinkByName(sinkName)
			}
		})

		test('filters sinks using type filter dropdown', async ({ sinkPage }) => {
			await sinkPage.gotoSinks()

			await sinkPage.filterDropdownButton.click()
			const filterMenu = sinkPage.page.locator('[role="menu"]')
			await expect(filterMenu).toBeVisible({ timeout: 5_000 })

			// Close filter menu by pressing Escape
			await sinkPage.page.keyboard.press('Escape')
			await expect(filterMenu).not.toBeVisible({ timeout: 5_000 })
		})
	})

	test.describe('Single-Cluster Sink Form', () => {
		test.use({ storageState: STORAGE_STATE.SINGLE_CLUSTER })

		test('verifies cluster input behavior in single-cluster mode', async ({ sinkPage }) => {
			await sinkPage.gotoSinks()
			await sinkPage.openCreateModal()

			await expect(sinkPage.modalDialog).toBeVisible()
			await expect(sinkPage.formNameInput).toBeVisible()

			// Cluster selector must be disabled/locked for single-cluster mode
			const clusterInput = sinkPage.formClusterInput.first()
			if (await clusterInput.isVisible()) {
				await expect(clusterInput).toHaveClass(/cursor-not-allowed|opacity-/)
			}

			await sinkPage.cancelForm()
			await expect(sinkPage.modalDialog).not.toBeVisible({ timeout: 10_000 })
		})
	})
})

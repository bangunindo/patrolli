// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { test, expect, STORAGE_STATE } from '../../fixtures/test-fixtures'

test.describe('Pipeline Management Feature', () => {
	test.describe('Multi-Cluster Session (Keycloak SSO)', () => {
		test('renders pipeline dashboard with table headers, action buttons, and table structure @smoke', async ({
			pipelinePage
		}) => {
			await pipelinePage.gotoPipelineList()

			// Verify heading and action buttons
			await expect(pipelinePage.heading).toBeVisible()
			await expect(pipelinePage.createPipelineButton).toBeVisible()
			await expect(pipelinePage.searchInput).toBeVisible()

			// Verify table headers
			await expect(pipelinePage.table).toBeVisible()
			const tableHeader = pipelinePage.table.locator('thead')
			await expect(tableHeader.getByText(/pipeline name/i)).toBeVisible()
			await expect(tableHeader.getByText(/status/i)).toBeVisible()
			await expect(tableHeader.getByText(/access/i)).toBeVisible()
			await expect(tableHeader.getByText(/assigned resources/i)).toBeVisible()
			await expect(tableHeader.getByText(/last update/i)).toBeVisible()
		})

		test('validates required fields in create pipeline wizard and handles cancellation', async ({
			pipelinePage
		}) => {
			await pipelinePage.gotoPipelineList()

			// Open Create Pipeline Modal
			await pipelinePage.createPipelineButton.click()
			await expect(pipelinePage.modalDialog).toBeVisible()

			// Attempt to submit empty form
			await pipelinePage.formSaveButton.click()

			// Expect validation alert
			const errorAlert = pipelinePage.modalDialog.locator('.bg-rose-50, .text-rose-700')
			await expect(errorAlert).toBeVisible()
			await expect(errorAlert).toContainText(/pipeline name is required/i)

			// Fill only name without hosts
			await pipelinePage.formNameInput.fill('temp_validation_pipe')
			await pipelinePage.formSaveButton.click()
			await expect(errorAlert).toBeVisible()
			await expect(errorAlert).toContainText(/at least one host must be assigned/i)

			// Cancel form and ensure closed
			await pipelinePage.formCancelButton.click()
			await expect(pipelinePage.modalDialog).not.toBeVisible()
		})

		test('manages complete pipeline lifecycle: create, search, edit metadata, toggle status, and delete', async ({
			pipelinePage
		}) => {
			const timestamp = Date.now()
			const pipelineName = `e2e_mgmt_${timestamp}`
			const editedName = `${pipelineName}_edited`

			await pipelinePage.gotoPipelineList()

			try {
				// 1. Create Pipeline
				await pipelinePage.createPipelineButton.click()
				await expect(pipelinePage.modalDialog).toBeVisible()

				await pipelinePage.fillCreatePipeline({
					name: pipelineName,
					access: 'public'
				})

				const createPromise = pipelinePage.page.waitForResponse(
					(res) =>
						res.url().includes('/api/pipelines') &&
						res.request().method() === 'POST' &&
						res.status() === 201,
					{ timeout: 15000 }
				)
				await pipelinePage.formSaveButton.click()
				await createPromise
				await expect(pipelinePage.modalDialog).not.toBeVisible()

				// 2. Search & Verify in Table
				await pipelinePage.searchPipeline(pipelineName)
				const pipelineRow = pipelinePage.getPipelineRow(pipelineName)
				await expect(pipelineRow).toBeVisible()
				await expect(pipelineRow.getByText('Public', { exact: true })).toBeVisible()

				// 3. Edit Metadata
				const detailPromise = pipelinePage.page
					.waitForResponse(
						(res) =>
							res.url().includes('/api/pipelines/') &&
							res.request().method() === 'GET' &&
							res.status() === 200,
						{ timeout: 10000 }
					)
					.catch(() => null)
				await pipelinePage.openPipelineAction(pipelineName, 'Edit')
				await detailPromise
				await expect(pipelinePage.modalDialog).toBeVisible()

				await pipelinePage.formNameInput.fill(editedName)
				await pipelinePage.formAccessPrivateRadio.click()

				const updatePromise = pipelinePage.page.waitForResponse(
					(res) =>
						res.url().includes('/api/pipelines') &&
						res.request().method() === 'PUT' &&
						res.status() < 400,
					{ timeout: 15000 }
				)
				await pipelinePage.formSaveButton.scrollIntoViewIfNeeded()
				await pipelinePage.formSaveButton.click()
				await updatePromise
				await expect(pipelinePage.modalDialog).not.toBeVisible()

				// Search edited pipeline
				await pipelinePage.searchPipeline(editedName)
				const editedRow = pipelinePage.getPipelineRow(editedName)
				await expect(editedRow).toBeVisible()
				await expect(editedRow.getByText('Private', { exact: true })).toBeVisible()

				// 4. Toggle Status (Stop Pipeline)
				await pipelinePage.openPipelineAction(editedName, 'Stop')
				await expect(pipelinePage.modalStatus).toBeVisible()

				const stopPromise = pipelinePage.page.waitForResponse(
					(res) =>
						res.url().includes('/api/pipelines') &&
						res.request().method() === 'PUT' &&
						res.status() < 400,
					{ timeout: 15000 }
				)
				await pipelinePage.modalStatusConfirmButton.click()
				await stopPromise
				await expect(pipelinePage.modalStatus).not.toBeVisible()

				// Verify status badge changed to Stopped
				await expect(editedRow.getByText('Stopped', { exact: true })).toBeVisible()

				// 5. Toggle Status (Start Pipeline)
				await pipelinePage.openPipelineAction(editedName, 'Start')
				await expect(pipelinePage.modalStatus).toBeVisible()

				const startPromise = pipelinePage.page.waitForResponse(
					(res) =>
						res.url().includes('/api/pipelines') &&
						res.request().method() === 'PUT' &&
						res.status() < 400,
					{ timeout: 15000 }
				)
				await pipelinePage.modalStatusConfirmButton.click()
				await startPromise
				await expect(pipelinePage.modalStatus).not.toBeVisible()

				// Verify status badge changed to Running
				await expect(editedRow.getByText('Running', { exact: true })).toBeVisible()

				// 6. Delete Pipeline
				await pipelinePage.openPipelineAction(editedName, 'Delete')
				await expect(pipelinePage.modalDelete).toBeVisible()

				const deletePromise = pipelinePage.page.waitForResponse(
					(res) =>
						res.url().includes('/api/pipelines') &&
						res.request().method() === 'DELETE' &&
						res.status() < 400,
					{ timeout: 15000 }
				)
				await pipelinePage.modalDeleteConfirmButton.click()
				await deletePromise
				await expect(pipelinePage.modalDelete).not.toBeVisible()

				// Verify row is removed
				await pipelinePage.searchPipeline(editedName)
				await expect(pipelinePage.getPipelineRow(editedName)).not.toBeVisible()
			} finally {
				// Fallback cleanup if test failed midway
				try {
					await pipelinePage.searchPipeline(pipelineName)
					let row = pipelinePage.getPipelineRow(pipelineName)
					if (!(await row.isVisible())) {
						await pipelinePage.searchPipeline(editedName)
						row = pipelinePage.getPipelineRow(editedName)
					}
					if (await row.isVisible()) {
						await pipelinePage.openPipelineAction(
							(await row.locator('.font-semibold').textContent()) || pipelineName,
							'Delete'
						)
						if (await pipelinePage.modalDelete.isVisible()) {
							await pipelinePage.modalDeleteConfirmButton.click()
						}
					}
				} catch {
					// Silent fallback cleanup
				}
			}
		})

		test('navigates to pipeline page from navbar on desktop and mobile', async ({
			pipelinePage
		}) => {
			// Start from another page
			await pipelinePage.page.goto('/cluster')
			await pipelinePage.page.waitForLoadState('domcontentloaded')

			// Navigate to Pipeline via navbar
			await pipelinePage.navigateFromNavbar()

			// Verify heading and URL
			await expect(pipelinePage.heading).toBeVisible()
			expect(pipelinePage.page.url()).toContain('/pipeline')
		})
	})

	test.describe('Single-Cluster Session (Local Login)', () => {
		test.use({ storageState: STORAGE_STATE.SINGLE_CLUSTER })

		test('locks cluster selector in single-cluster session', async ({ pipelinePage }) => {
			await pipelinePage.gotoPipelineList()

			// Open Create Pipeline Modal
			await pipelinePage.createPipelineButton.click()
			await expect(pipelinePage.modalDialog).toBeVisible()

			// Verify cluster selector is locked/disabled in single-cluster mode
			const clusterInput = pipelinePage.formClusterInput.first()
			if (await clusterInput.isVisible()) {
				await expect(clusterInput).toHaveClass(/cursor-not-allowed|opacity-50/)
			}

			// Close modal
			await pipelinePage.formCancelButton.click()
			await expect(pipelinePage.modalDialog).not.toBeVisible()
		})
	})
})

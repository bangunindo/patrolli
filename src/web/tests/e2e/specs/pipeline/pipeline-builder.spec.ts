// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { test, expect } from '../../fixtures/test-fixtures'

test.describe('Pipeline Visual DAG Builder Canvas', () => {
	const timestamp = Date.now()
	const isolatedPipelineName = `e2e_builder_${timestamp}`
	const seedHostName = `e2e-builder-host-${timestamp}`

	test('creates isolated pipeline and manages complete visual DAG builder flow', async ({
		pipelinePage,
		pipelineBuilderPage,
		apiHelper
	}) => {
		await apiHelper.createHost(seedHostName)
		await pipelinePage.gotoPipelineList()

		try {
			// 1. Setup Isolated Pipeline
			await pipelinePage.createPipelineButton.click()
			await expect(pipelinePage.modalDialog).toBeVisible()

			await pipelinePage.fillCreatePipeline({
				name: isolatedPipelineName,
				access: 'public',
				hostName: seedHostName
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

			// 2. Open Builder Canvas
			await pipelinePage.searchPipeline(isolatedPipelineName)
			await pipelineBuilderPage.openPipelineBuilderByName(isolatedPipelineName)

			// Verify canvas and header controls
			await expect(pipelineBuilderPage.canvas).toBeVisible({ timeout: 15000 })
			await expect(pipelineBuilderPage.pipelineTitle).toContainText(isolatedPipelineName)
			await expect(pipelineBuilderPage.saveButton).toBeVisible()
			await expect(pipelineBuilderPage.saveButton).toContainText(/saved/i)
			await expect(pipelineBuilderPage.autoLayoutButton).toBeVisible()

			// Add Source Node to empty canvas
			await pipelineBuilderPage.addSourceNode()
			await expect(pipelineBuilderPage.sourceNode).toBeVisible()

			// 3. Add Processor Group & Sink Nodes to Canvas
			await pipelineBuilderPage.addProcessorGroupNode()
			await expect(pipelineBuilderPage.processorNodes.first()).toBeVisible()

			await pipelineBuilderPage.addSinkNode()
			await expect(pipelineBuilderPage.sinkNodes.first()).toBeVisible()

			// Canvas is now dirty
			await expect(pipelineBuilderPage.saveButton).toContainText(/save changes/i)

			// 4. Trigger Auto-Layout and Save Graph
			await pipelineBuilderPage.triggerAutoLayout()

			const savePromise = pipelineBuilderPage.page.waitForResponse(
				(res) =>
					res.url().includes('/canvases/') &&
					res.url().includes('/graph') &&
					res.request().method() === 'PUT' &&
					res.status() === 200,
				{ timeout: 15000 }
			)
			await pipelineBuilderPage.clickSave()
			await savePromise

			// Verify save button transitions back to Saved
			await expect(pipelineBuilderPage.saveButton).toContainText(/saved/i)

			// 5. Test Unsaved Changes Guard
			// Add another node to make canvas dirty again
			await pipelineBuilderPage.addProcessorGroupNode()
			await expect(pipelineBuilderPage.saveButton).toContainText(/save changes/i)

			// Attempt navigation back to pipeline list
			await pipelineBuilderPage.backButton.click()

			// Guard modal should appear
			await expect(pipelineBuilderPage.modalUnsaved).toBeVisible()

			// Cancel leave: stays in builder
			await pipelineBuilderPage.buttonCancelLeave.click()
			await expect(pipelineBuilderPage.modalUnsaved).not.toBeVisible()
			expect(pipelineBuilderPage.page.url()).toContain('/builder/')

			// Discard and leave: proceeds to /pipeline
			await pipelineBuilderPage.backButton.click()
			await expect(pipelineBuilderPage.modalUnsaved).toBeVisible()
			await pipelineBuilderPage.buttonDiscardLeave.click()
			await pipelineBuilderPage.page.waitForURL(/\/pipeline(?!\/)/)
			expect(pipelineBuilderPage.page.url()).toMatch(/\/pipeline$/)
		} finally {
			// Cleanup isolated pipeline and seeded host
			try {
				await pipelinePage.gotoPipelineList()
				await pipelinePage.searchPipeline(isolatedPipelineName)
				const row = pipelinePage.getPipelineRow(isolatedPipelineName)
				if (await row.isVisible()) {
					await pipelinePage.openPipelineAction(isolatedPipelineName, 'Delete')
					if (await pipelinePage.modalDelete.isVisible()) {
						await pipelinePage.modalDeleteConfirmButton.click()
						await expect(pipelinePage.modalDelete).not.toBeVisible({ timeout: 10000 })
					}
				}
			} catch {
				// Silent cleanup
			}
			await apiHelper.deletePipelineByName(isolatedPipelineName)
			await apiHelper.deleteHostByName(seedHostName)
		}
	})
})

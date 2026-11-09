// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { test, expect, STORAGE_STATE } from '../../fixtures/test-fixtures'

test.describe('Configuration Management - Templates', () => {
	test.describe('Authenticated Session (Multi-Cluster)', () => {
		test('renders templates page with table, search, and action header @smoke', async ({
			templatePage
		}) => {
			await templatePage.gotoTemplates()

			await expect(templatePage.createTemplateButton).toBeVisible()
			await expect(templatePage.searchInput).toBeVisible()
			await expect(templatePage.table).toBeVisible()
		})

		test('validates required fields in create template form and handles modal cancel', async ({
			templatePage
		}) => {
			await templatePage.gotoTemplates()
			await templatePage.openCreateModal()

			await expect(templatePage.modalDialog).toBeVisible()
			await expect(templatePage.formSaveButton).toBeVisible()

			// Attempt saving without required name: validation keeps dialog open
			await templatePage.submitForm()
			await expect(templatePage.modalDialog).toBeVisible()

			// Dismiss dialog via cancel button
			await templatePage.formCancelButton.click()
			await expect(templatePage.modalDialog).not.toBeVisible({ timeout: 10_000 })
		})

		test('creates, views detail, edits, and deletes a template lifecycle', async ({
			templatePage,
			apiHelper
		}) => {
			const templateName = `e2e-tmpl-${Date.now()}`
			const updatedTemplateName = `${templateName}-mod`

			try {
				await templatePage.gotoTemplates()
				await templatePage.openCreateModal()
				await templatePage.fillTemplateName(templateName)
				await templatePage.submitForm()
				await expect(templatePage.modalDialog).not.toBeVisible({ timeout: 15_000 })

				await templatePage.searchTemplate(templateName)
				const templateRow = await templatePage.getRowByName(templateName)
				await expect(templateRow).toBeVisible({ timeout: 10_000 })

				await templatePage.openDetail(templateName)
				await expect(templatePage.modalDetail).toBeVisible({ timeout: 10_000 })
				await expect(templatePage.modalDetail).toContainText(templateName)
				const closeDetailBtn = templatePage.modalDetail
					.getByRole('button', { name: /^close$/i })
					.first()
				await closeDetailBtn.click()
				await expect(templatePage.modalDetail).not.toBeVisible({ timeout: 5_000 })

				await templatePage.openEdit(templateName)
				await templatePage.fillTemplateName(updatedTemplateName)
				await templatePage.submitForm()
				await expect(templatePage.modalDialog).not.toBeVisible({ timeout: 15_000 })

				await templatePage.clearSearch()
				await templatePage.searchTemplate(updatedTemplateName)
				const updatedRow = await templatePage.getRowByName(updatedTemplateName)
				await expect(updatedRow).toBeVisible({ timeout: 10_000 })

				await templatePage.deleteTemplate(updatedTemplateName)
				await templatePage.clearSearch()
				await templatePage.searchTemplate(updatedTemplateName)
				const rowAfterDelete = await templatePage.getRowByName(updatedTemplateName)
				await expect(rowAfterDelete).not.toBeVisible({ timeout: 10_000 })
			} finally {
				await apiHelper.deleteTemplateByName(updatedTemplateName)
				await apiHelper.deleteTemplateByName(templateName)
			}
		})

		test('navigates to template items subpage and back', async ({
			templatePage,
			apiHelper
		}) => {
			const templateName = `e2e-tmpl-sub-${Date.now()}`

			try {
				await templatePage.gotoTemplates()
				await templatePage.openCreateModal()
				await templatePage.fillTemplateName(templateName)
				await templatePage.submitForm()
				await expect(templatePage.modalDialog).not.toBeVisible({ timeout: 15_000 })

				await templatePage.searchTemplate(templateName)
				const row = await templatePage.getRowByName(templateName)
				await expect(row).toBeVisible({ timeout: 10_000 })
				await templatePage.clickItemsLink(templateName)

				await expect(templatePage.page).toHaveURL(/.*\/items/)
				const heading = templatePage.page.getByRole('heading', { name: /template items/i })
				await expect(heading).toBeVisible({ timeout: 10_000 })
				const createItemBtn = templatePage.page.getByRole('button', {
					name: /create item for template/i
				})
				await expect(createItemBtn).toBeVisible({ timeout: 10_000 })

				await templatePage.goBackFromItems()
				await expect(templatePage.page).toHaveURL(/\/configuration\/template(\/)?(\?.*)?$/)

				await templatePage.searchTemplate(templateName)
				await templatePage.deleteTemplate(templateName)
			} finally {
				await apiHelper.deleteTemplateByName(templateName)
			}
		})

		test('manages template linking and handles unlink confirmation modal lifecycle', async ({
			templatePage,
			apiHelper
		}) => {
			const parentName = `e2e-tp-${Date.now()}`
			const childName = `e2e-tc-${Date.now()}`

			try {
				await templatePage.gotoTemplates()
				await templatePage.openCreateModal()
				await templatePage.fillTemplateName(parentName)
				await templatePage.submitForm()
				await expect(templatePage.modalDialog).not.toBeVisible({ timeout: 15_000 })
				await templatePage.searchTemplate(parentName)
				await expect(await templatePage.getRowByName(parentName)).toBeVisible({
					timeout: 10_000
				})
				await templatePage.clearSearch()

				await templatePage.openCreateModal()
				await templatePage.fillTemplateName(childName)
				await templatePage.selectLinkTemplate(parentName)
				await templatePage.submitForm()
				await expect(templatePage.modalDialog).not.toBeVisible({ timeout: 15_000 })

				await templatePage.searchTemplate(childName)
				const childRow = await templatePage.getRowByName(childName)
				await expect(childRow).toBeVisible({ timeout: 10_000 })
				await expect(childRow).toContainText(parentName)

				await templatePage.openEdit(childName)
				await templatePage.selectLinkTemplate(parentName)
				await templatePage.submitForm()

				await expect(templatePage.modalUnlinkConfirm).toBeVisible({ timeout: 10_000 })
				await expect(templatePage.modalUnlinkConfirm).toContainText(parentName)

				await templatePage.unlinkConfirmButton.click()
				await expect(templatePage.modalUnlinkConfirm).not.toBeVisible({ timeout: 15_000 })
				await expect(templatePage.modalDialog).not.toBeVisible({ timeout: 15_000 })

				await templatePage.clearSearch()
				await templatePage.searchTemplate(childName)
				await templatePage.deleteTemplate(childName)

				await templatePage.clearSearch()
				await templatePage.searchTemplate(parentName)
				await templatePage.deleteTemplate(parentName)
			} finally {
				await apiHelper.deleteTemplateByName(childName)
				await apiHelper.deleteTemplateByName(parentName)
			}
		})
	})

	test.describe('Single-Cluster Template Form', () => {
		test.use({ storageState: STORAGE_STATE.SINGLE_CLUSTER })

		test('locks cluster selector in single-cluster session', async ({ templatePage }) => {
			await templatePage.gotoTemplates()
			await templatePage.openCreateModal()

			// Cluster selector must be disabled/locked for single-cluster mode
			const clusterInput = templatePage.formClusterInput.first()
			if (await clusterInput.isVisible()) {
				await expect(clusterInput).toHaveClass(/cursor-not-allowed|opacity-50/)
			}

			await templatePage.formCancelButton.click()
			await expect(templatePage.modalDialog).not.toBeVisible({ timeout: 10_000 })
		})
	})
})

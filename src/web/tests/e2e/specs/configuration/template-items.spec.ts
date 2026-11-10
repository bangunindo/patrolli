// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { expect, test } from '../../fixtures/test-fixtures'

test.describe('Configuration Management - Template Items', () => {
	test.describe('Authenticated Session (Multi-Cluster)', () => {
		test('renders template items subpage with header, search, and action button', async ({
			templatePage,
			templateItemsPage,
			apiHelper
		}) => {
			const parentName = `e2e-tp-items-${Date.now()}`

			try {
				// 1. Setup parent template
				await templatePage.gotoTemplates()
				await templatePage.openCreateModal()
				await templatePage.fillTemplateName(parentName)
				await templatePage.submitForm()
				await expect(templatePage.modalDialog).not.toBeVisible({ timeout: 15_000 })

				await templatePage.searchTemplate(parentName)
				await expect(await templatePage.getRowByName(parentName)).toBeVisible({
					timeout: 10_000
				})
				await templatePage.clickItemsLink(parentName)

				// 2. Verify Template Items Subpage Elements
				await expect(templateItemsPage.page).toHaveURL(
					/.*\/configuration\/template\/.*\/items/
				)
				await expect(
					templateItemsPage.page.getByRole('heading', { name: /template items/i })
				).toBeVisible({ timeout: 10_000 })
				await expect(templateItemsPage.page.getByText(`Items for template:`)).toBeVisible({
					timeout: 10_000
				})
				await expect(templateItemsPage.page.getByText(parentName)).toBeVisible({
					timeout: 10_000
				})
				await expect(templateItemsPage.createItemButton).toBeVisible({ timeout: 10_000 })
				await expect(templateItemsPage.searchInput).toBeVisible({ timeout: 10_000 })
				await expect(templateItemsPage.table).toBeVisible({ timeout: 10_000 })
			} finally {
				await apiHelper.deleteTemplateByName(parentName)
			}
		})

		test('validates required fields in create template item form and handles modal cancel', async ({
			templatePage,
			templateItemsPage,
			apiHelper
		}) => {
			const parentName = `e2e-tp-val-${Date.now()}`

			try {
				// 1. Setup parent template
				await templatePage.gotoTemplates()
				await templatePage.openCreateModal()
				await templatePage.fillTemplateName(parentName)
				await templatePage.submitForm()
				await expect(templatePage.modalDialog).not.toBeVisible({ timeout: 15_000 })

				await templatePage.searchTemplate(parentName)
				await templatePage.clickItemsLink(parentName)

				// 2. Open Create Item Modal
				await templateItemsPage.openCreateModal()
				await expect(templateItemsPage.modalDialog).toBeVisible({ timeout: 5_000 })

				// 3. Submit empty form and verify validation blocks closure
				await templateItemsPage.submitForm()
				await expect(templateItemsPage.modalDialog).toBeVisible({ timeout: 5_000 })

				// 4. Cancel and verify modal closes
				await templateItemsPage.formCancelButton.click()
				await expect(templateItemsPage.modalDialog).not.toBeVisible({ timeout: 10_000 })
			} finally {
				await apiHelper.deleteTemplateByName(parentName)
			}
		})

		test('creates, views detail, edits, and deletes a template item lifecycle', async ({
			templatePage,
			templateItemsPage,
			apiHelper
		}) => {
			const parentName = `e2e-tp-crud-${Date.now()}`
			const itemName = `e2e-item-${Date.now()}`
			const updatedName = `${itemName}-mod`

			try {
				await templatePage.gotoTemplates()
				await templatePage.openCreateModal()
				await templatePage.fillTemplateName(parentName)
				await templatePage.submitForm()
				await expect(templatePage.modalDialog).not.toBeVisible({ timeout: 15_000 })

				await templatePage.searchTemplate(parentName)
				await templatePage.clickItemsLink(parentName)

				await templateItemsPage.openCreateModal()
				await templateItemsPage.fillItemForm({
					name: itemName,
					schedule: '60',
					timeout: '5000',
					oid: '1.3.6.1.2.1.1.1.0'
				})
				await templateItemsPage.submitForm()
				await expect(templateItemsPage.modalDialog).not.toBeVisible({ timeout: 15_000 })

				await templateItemsPage.searchItem(itemName)
				const itemRow = await templateItemsPage.getRowByName(itemName)
				await expect(itemRow).toBeVisible({ timeout: 10_000 })
				await expect(itemRow).toContainText('Active')
				await expect(itemRow).toContainText(/snmp/i)

				await templateItemsPage.openDetail(itemName)
				await expect(templateItemsPage.modalDetail).toBeVisible({ timeout: 5_000 })
				await expect(templateItemsPage.modalDetail).toContainText(itemName)
				await expect(templateItemsPage.modalDetail).toContainText(parentName)
				await expect(templateItemsPage.modalDetail).toContainText(
					/sysDescr|1\.3\.6\.1\.2\.1\.1\.1\.0/
				)
				await expect(templateItemsPage.modalDetail).toContainText(/snmp/i)
				await templateItemsPage.modalDetail
					.getByRole('button', { name: /^close$/i })
					.first()
					.click()
				await expect(templateItemsPage.modalDetail).not.toBeVisible({ timeout: 10_000 })

				await templateItemsPage.openEdit(itemName)
				await templateItemsPage.fillItemForm({
					name: updatedName,
					schedule: '120',
					oid: '1.3.6.1.2.1.1.1.0'
				})
				await templateItemsPage.submitForm()
				await expect(templateItemsPage.modalDialog).not.toBeVisible({ timeout: 15_000 })

				await templateItemsPage.clearSearch()
				await templateItemsPage.searchItem(updatedName)
				const updatedRow = await templateItemsPage.getRowByName(updatedName)
				await expect(updatedRow).toBeVisible({ timeout: 10_000 })

				await templateItemsPage.deleteItem(updatedName)
				await templateItemsPage.clearSearch()
				await templateItemsPage.searchItem(updatedName)
				await expect(await templateItemsPage.getRowByName(updatedName)).not.toBeVisible({
					timeout: 10_000
				})
			} finally {
				await apiHelper.deleteItemByName(updatedName)
				await apiHelper.deleteItemByName(itemName)
				await apiHelper.deleteTemplateByName(parentName)
			}
		})

		test('navigates back to templates list page via header back button', async ({
			templatePage,
			templateItemsPage,
			apiHelper
		}) => {
			const parentName = `e2e-tp-nav-${Date.now()}`

			try {
				// 1. Setup parent template
				await templatePage.gotoTemplates()
				await templatePage.openCreateModal()
				await templatePage.fillTemplateName(parentName)
				await templatePage.submitForm()
				await expect(templatePage.modalDialog).not.toBeVisible({ timeout: 15_000 })

				await templatePage.searchTemplate(parentName)
				await templatePage.clickItemsLink(parentName)

				// 2. Verify subpage loaded and click back
				await expect(templateItemsPage.page).toHaveURL(/.*\/items/)
				await templateItemsPage.goBack()

				// 3. Verify returned to Templates page
				await expect(templateItemsPage.page).toHaveURL(
					/\/configuration\/template(\/)?(\?.*)?$/
				)
				await expect(templatePage.createTemplateButton).toBeVisible({ timeout: 10_000 })
			} finally {
				await apiHelper.deleteTemplateByName(parentName)
			}
		})

		test('verifies template context form isolation (no host, endpoint, or listener fields)', async ({
			templatePage,
			templateItemsPage,
			apiHelper
		}) => {
			const parentName = `e2e-tp-iso-${Date.now()}`

			try {
				// 1. Setup parent template
				await templatePage.gotoTemplates()
				await templatePage.openCreateModal()
				await templatePage.fillTemplateName(parentName)
				await templatePage.submitForm()
				await expect(templatePage.modalDialog).not.toBeVisible({ timeout: 15_000 })

				await templatePage.searchTemplate(parentName)
				await templatePage.clickItemsLink(parentName)

				// 2. Open form and assert isolation
				await templateItemsPage.openCreateModal()
				const formLocator = templateItemsPage.modalDialog.locator('#item-form')

				// In template items context, Host, Cluster, Endpoint, and Listener inputs should not be rendered
				await expect(formLocator.locator('.app-input-host')).toHaveCount(0)
				await expect(formLocator.locator('.app-input-cluster')).toHaveCount(0)
				await expect(formLocator.locator('.app-input-endpoint')).toHaveCount(0)
				await expect(formLocator.locator('.app-input-listener')).toHaveCount(0)

				// 3. Cancel form
				await templateItemsPage.formCancelButton.click()
				await expect(templateItemsPage.modalDialog).not.toBeVisible({ timeout: 10_000 })
			} finally {
				await apiHelper.deleteTemplateByName(parentName)
			}
		})
	})
})

// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { test, expect } from '../../fixtures/test-fixtures'

test.describe('Configuration Management - Items', () => {
	test.describe('Authenticated Session (Multi-Cluster)', () => {
		test('renders items management page with table, search, and action header @smoke', async ({
			itemsPage
		}) => {
			await itemsPage.gotoItems()

			await expect(itemsPage.createItemButton).toBeVisible()
			await expect(itemsPage.searchInput).toBeVisible()
			await expect(itemsPage.filterDropdownButton).toBeVisible()
			await expect(itemsPage.table).toBeVisible()

			// Test opening filter dropdown
			await itemsPage.filterDropdownButton.click()
			const filterMenu = itemsPage.page.locator('[role="menu"]')
			await expect(filterMenu).toBeVisible({ timeout: 5_000 })

			// Close filter menu by pressing Escape
			await itemsPage.page.keyboard.press('Escape')
			await expect(filterMenu).not.toBeVisible({ timeout: 5_000 })
		})

		test('validates required fields in create item form and handles modal cancel', async ({
			itemsPage
		}) => {
			await itemsPage.gotoItems()
			await itemsPage.openCreateModal()

			await expect(itemsPage.modalDialog).toBeVisible()
			await expect(itemsPage.formNameInput).toBeVisible()
			await expect(itemsPage.formSaveButton).toBeVisible()

			// Submit empty form: validation keeps modal open
			await itemsPage.submitForm()
			await expect(itemsPage.modalDialog).toBeVisible()

			// Cancel form dialog
			await itemsPage.cancelForm()
			await expect(itemsPage.modalDialog).not.toBeVisible()
		})

		test('creates, views detail, inspects trends drawer, and edits a host item lifecycle', async ({
			hostPage,
			endpointPage,
			itemsPage,
			apiHelper
		}) => {
			test.setTimeout(90_000)
			const hostName = `e2e-item-host-${Date.now()}`
			const endpointName = `e2e-item-ep-${Date.now()}`
			const itemName = `e2e-hitem-${Date.now()}`
			const updatedItemName = `${itemName}-mod`

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
				await endpointPage.fillSnmpV1OrV2c('192.168.10.50:161', 'public')
				await endpointPage.submitForm()
				await expect(endpointPage.modalDialog).not.toBeVisible({ timeout: 15_000 })

				await itemsPage.gotoItems()
				await itemsPage.openCreateModal()
				await itemsPage.selectHost(hostName)
				await itemsPage.selectEndpoint(endpointName)
				await itemsPage.fillItemForm({
					name: itemName,
					schedule: '30',
					timeout: '1000',
					oid: '1.3.6.1.2.1.1.1.0'
				})
				await itemsPage.submitForm()
				await expect(itemsPage.modalDialog).not.toBeVisible({ timeout: 15_000 })

				await itemsPage.searchItem(itemName)
				const itemRow = await itemsPage.getRowByName(itemName)
				await expect(itemRow).toBeVisible({ timeout: 10_000 })
				await expect(itemRow).toContainText(/snmp/i)
				await expect(itemRow).toContainText(hostName)

				await itemsPage.openDetail(itemName)
				await expect(itemsPage.modalDetail).toBeVisible({ timeout: 10_000 })
				await expect(itemsPage.modalDetail).toContainText(itemName)
				await expect(itemsPage.modalDetail).toContainText(/host specific/i)

				const closeDetailBtn = itemsPage.modalDetail
					.getByRole('button', { name: /^close$/i })
					.first()
				await closeDetailBtn.click()
				await expect(itemsPage.modalDetail).not.toBeVisible({ timeout: 5_000 })

				await itemsPage.openTrends(itemName)
				await expect(itemsPage.drawer).toBeVisible({ timeout: 10_000 })
				await expect(itemsPage.drawer).toContainText(itemName)

				await itemsPage.closeDrawer()
				await expect(itemsPage.drawer).not.toBeVisible({ timeout: 5_000 })

				await itemsPage.openEdit(itemName)
				await itemsPage.fillItemForm({
					name: updatedItemName,
					schedule: '60',
					oid: '1.3.6.1.2.1.1.1.0'
				})
				await itemsPage.submitForm()
				await expect(itemsPage.modalDialog).not.toBeVisible({ timeout: 15_000 })

				await itemsPage.clearSearch()
				await itemsPage.searchItem(updatedItemName)
				const updatedRow = await itemsPage.getRowByName(updatedItemName)
				await expect(updatedRow).toBeVisible({ timeout: 10_000 })
				await expect(updatedRow).toContainText(/1 Minute/i)

				await itemsPage.deleteItem(updatedItemName)
				await itemsPage.clearSearch()
				await itemsPage.searchItem(updatedItemName)
				const rowAfterDelete = await itemsPage.getRowByName(updatedItemName)
				await expect(rowAfterDelete).not.toBeVisible({ timeout: 10_000 })
			} finally {
				await apiHelper.deleteItemByName(updatedItemName)
				await apiHelper.deleteItemByName(itemName)
				await apiHelper.deleteEndpointByName(endpointName)
				await apiHelper.deleteHostByName(hostName)
			}
		})

		test('verifies applied template item, template navigation link, detail, and override lifecycle', async ({
			hostPage,
			endpointPage,
			templatePage,
			templateItemsPage,
			itemsPage,
			page,
			apiHelper
		}) => {
			test.setTimeout(90_000)
			const hostName = `e2e-host-app-${Date.now()}`
			const endpointName = `e2e-ep-app-${Date.now()}`
			const tplName = `e2e-tpl-app-${Date.now()}`
			const tplItemName = `e2e-titem-${Date.now()}`
			const itemOverrideName = `${tplItemName}-override`

			try {
				await templatePage.gotoTemplates()
				await templatePage.openCreateModal()
				await templatePage.fillTemplateName(tplName)
				await templatePage.submitForm()
				await expect(templatePage.modalDialog).not.toBeVisible({ timeout: 15_000 })

				await templatePage.searchTemplate(tplName)
				await templatePage.clickItemsLink(tplName)
				await templateItemsPage.openCreateModal()
				await templateItemsPage.fillItemForm({
					name: tplItemName,
					schedule: '45',
					timeout: '2000',
					oid: '1.3.6.1.2.1.1.2.0'
				})
				await templateItemsPage.submitForm()
				await expect(templateItemsPage.modalDialog).not.toBeVisible({ timeout: 15_000 })

				await hostPage.gotoHosts()
				await hostPage.openCreateModal()
				await hostPage.fillHostName(hostName)
				await hostPage.selectTemplate(tplName)
				await hostPage.submitForm()
				await expect(hostPage.modalDialog).not.toBeVisible({ timeout: 15_000 })

				await endpointPage.gotoEndpoints()
				await endpointPage.openCreateModal()
				await endpointPage.fillEndpointName(endpointName)
				await endpointPage.selectHost(hostName)
				await endpointPage.fillSnmpV1OrV2c('192.168.10.51:161', 'public')
				await endpointPage.submitForm()
				await expect(endpointPage.modalDialog).not.toBeVisible({ timeout: 15_000 })

				await itemsPage.gotoItems()
				await itemsPage.searchItem(tplItemName)
				const appliedRow = await itemsPage.getRowByName(tplItemName)
				await expect(appliedRow).toBeVisible({ timeout: 15_000 })
				await expect(appliedRow).toContainText(hostName)
				await expect(appliedRow).toContainText(tplName)

				const tplLink = appliedRow.getByRole('button', { name: new RegExp(tplName, 'i') })
				await expect(tplLink).toBeVisible()
				await tplLink.click()
				await page.waitForURL(/\/configuration\/template\/[^/]+\/items/, {
					timeout: 10_000
				})
				await expect(page.getByRole('heading', { name: /template items/i })).toBeVisible()

				await itemsPage.gotoItems()
				await itemsPage.searchItem(tplItemName)

				await itemsPage.openDetail(tplItemName)
				await expect(itemsPage.modalDetail).toBeVisible({ timeout: 10_000 })
				await expect(itemsPage.modalDetail).toContainText(tplItemName)
				await expect(itemsPage.modalDetail).toContainText(/template/i)

				const closeDetailBtn = itemsPage.modalDetail
					.getByRole('button', { name: /^close$/i })
					.first()
				await closeDetailBtn.click()
				await expect(itemsPage.modalDetail).not.toBeVisible({ timeout: 5_000 })

				await itemsPage.openEdit(tplItemName)
				await expect(itemsPage.modalDialog).toContainText(/applied template/i)
				await itemsPage.selectEndpoint(endpointName)
				await itemsPage.fillItemForm({
					name: itemOverrideName,
					schedule: '120',
					oid: '1.3.6.1.2.1.1.2.0'
				})
				await itemsPage.submitForm()
				await expect(itemsPage.modalDialog).not.toBeVisible({ timeout: 15_000 })

				await itemsPage.clearSearch()
				await itemsPage.searchItem(itemOverrideName)
				const overriddenRow = await itemsPage.getRowByName(itemOverrideName)
				await expect(overriddenRow).toBeVisible({ timeout: 10_000 })
				await expect(overriddenRow).toContainText(/2 Minute/i)
			} finally {
				await apiHelper.deleteItemByName(itemOverrideName)
				await apiHelper.deleteItemByName(tplItemName)
				await apiHelper.deleteEndpointByName(endpointName)
				await apiHelper.deleteHostByName(hostName)
				await apiHelper.deleteTemplateByName(tplName)
			}
		})
	})
})

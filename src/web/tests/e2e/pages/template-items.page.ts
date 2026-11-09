// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { Locator, Page } from '@playwright/test'
import { BasePage } from './base.page'

export class TemplateItemsPage extends BasePage {
	readonly createItemButton: Locator
	readonly backButton: Locator
	readonly searchInput: Locator
	readonly table: Locator
	readonly tableRows: Locator
	readonly modalDialog: Locator
	readonly modalDetail: Locator
	readonly modalDeleteConfirm: Locator

	// Form fields
	readonly formNameInput: Locator
	readonly formScheduleInput: Locator
	readonly formTimeoutInput: Locator
	readonly formOidInput: Locator
	readonly formSaveButton: Locator
	readonly formCancelButton: Locator

	constructor(page: Page) {
		super(page)

		this.createItemButton = page.getByRole('button', { name: /create item for template/i })
		this.backButton = page
			.locator('div.flex.items-center.gap-3')
			.filter({ has: page.getByRole('heading', { name: /template items/i }) })
			.locator('button')
			.first()
		this.searchInput = page.getByPlaceholder(/search by name/i)
		this.table = page.locator('table')
		this.tableRows = page.locator('table tbody tr')
		this.modalDialog = page.locator('[role="dialog"]')
		this.modalDetail = page.locator('[role="dialog"]').filter({ hasText: /read one item/i })
		this.modalDeleteConfirm = page
			.locator('[role="dialog"]')
			.filter({ has: page.getByRole('heading', { name: /^delete$/i }) })

		this.formNameInput = page.locator('#item-form input').first()
		this.formScheduleInput = page.locator('#item-form').getByPlaceholder(/^30$/)
		this.formTimeoutInput = page.locator('#item-form').getByPlaceholder(/^1000$/)
		this.formOidInput = page.locator('#item-form').getByPlaceholder(/1\.3\.6\.1\.2\.1\.1\.1\.0/)
		this.formSaveButton = page.locator('#item-form').getByRole('button', { name: /^save$/i })
		this.formCancelButton = page
			.locator('#item-form')
			.getByRole('button', { name: /^cancel$/i })
	}

	async gotoTemplateItems(templateId: string, clusterName: string = 'patrolli') {
		await this.goto(`/configuration/template/${templateId}/items?cluster=${clusterName}`)
		await this.page.waitForLoadState('domcontentloaded')
		await this.createItemButton.waitFor({ state: 'visible', timeout: 20_000 })
	}

	async openCreateModal() {
		await this.createItemButton.click()
		await this.modalDialog.waitFor({ state: 'visible', timeout: 5_000 })
	}

	async fillItemForm(data: { name?: string; schedule?: string; timeout?: string; oid?: string }) {
		if (data.name !== undefined) {
			await this.formNameInput.fill(data.name)
			await this.formNameInput.dispatchEvent('input')
			await this.formNameInput.blur()
		}
		if (data.schedule !== undefined) {
			await this.formScheduleInput.fill(data.schedule)
			await this.formScheduleInput.dispatchEvent('input')
			await this.formScheduleInput.blur()
		}
		if (data.timeout !== undefined) {
			await this.formTimeoutInput.fill(data.timeout)
			await this.formTimeoutInput.dispatchEvent('input')
			await this.formTimeoutInput.blur()
		}
		if (data.oid !== undefined) {
			await this.formOidInput.fill(data.oid)
			await this.formOidInput.dispatchEvent('input')
			await this.formOidInput.blur()
		}
	}

	async submitForm() {
		await this.formSaveButton.click()
	}

	async searchItem(name: string) {
		const currentVal = await this.searchInput.inputValue()
		if (currentVal === name) {
			await this.table.waitFor({ state: 'visible' })
			return
		}

		const searchPromise = this.page
			.waitForResponse(
				(res) => {
					try {
						const url = new URL(res.url())
						return (
							url.pathname.includes('/api/items') &&
							url.searchParams.get('name') === name &&
							res.status() < 400
						)
					} catch {
						return false
					}
				},
				{ timeout: 10_000 }
			)
			.catch(() => null)
		await this.searchInput.fill(name)
		await searchPromise
		await this.table.waitFor({ state: 'visible' })
	}

	async clearSearch() {
		const currentVal = await this.searchInput.inputValue()
		if (!currentVal) {
			await this.table.waitFor({ state: 'visible' })
			return
		}

		const clearPromise = this.page
			.waitForResponse(
				(res) => {
					try {
						const url = new URL(res.url())
						return (
							url.pathname.includes('/api/items') &&
							!url.searchParams.get('name') &&
							res.status() < 400
						)
					} catch {
						return false
					}
				},
				{ timeout: 10_000 }
			)
			.catch(() => null)
		await this.searchInput.fill('')
		await clearPromise
		await this.table.waitFor({ state: 'visible' })
	}

	async getRowByName(name: string): Promise<Locator> {
		return this.tableRows.filter({ hasText: name }).first()
	}

	async openDetail(itemName: string) {
		const row = await this.getRowByName(itemName)
		const actionButton = row.locator('button').last()
		await actionButton.click()

		const detailOption = this.page.getByRole('menuitem', { name: /view detail/i })
		await detailOption.click()
		await this.modalDetail.waitFor({ state: 'visible', timeout: 5_000 })
	}

	async openEdit(itemName: string) {
		const row = await this.getRowByName(itemName)
		const actionButton = row.locator('button').last()
		await actionButton.click()

		const editOption = this.page.getByRole('menuitem', { name: /edit template item/i })
		await editOption.click()
		await this.modalDialog.waitFor({ state: 'visible', timeout: 5_000 })
	}

	async deleteItem(itemName: string) {
		const row = await this.getRowByName(itemName)
		const actionButton = row.locator('button').last()
		await actionButton.click()

		const deleteOption = this.page.getByRole('menuitem', { name: /delete template item/i })
		await deleteOption.click()
		await this.modalDeleteConfirm.waitFor({ state: 'visible', timeout: 5_000 })

		const confirmDeleteBtn = this.modalDeleteConfirm.getByRole('button', { name: /^delete$/i })
		await confirmDeleteBtn.click()
		await this.modalDeleteConfirm.waitFor({ state: 'hidden', timeout: 15_000 })
	}

	async goBack() {
		await this.backButton.click()
		await this.page.waitForURL(/\/configuration\/template(\/)?(\?.*)?$/, { timeout: 10_000 })
	}
}

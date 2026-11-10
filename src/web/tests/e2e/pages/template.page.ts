// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { Locator, Page } from '@playwright/test'
import { BasePage } from './base.page'

export class TemplatePage extends BasePage {
	readonly createTemplateButton: Locator
	readonly searchInput: Locator
	readonly table: Locator
	readonly tableRows: Locator
	readonly modalDialog: Locator
	readonly modalDetail: Locator
	readonly modalDeleteConfirm: Locator
	readonly modalUnlinkConfirm: Locator

	// Form fields
	readonly formNameInput: Locator
	readonly formClusterInput: Locator
	readonly formLinkTemplatesTrigger: Locator
	readonly formSaveButton: Locator
	readonly formCancelButton: Locator

	// Unlink Modal fields
	readonly unlinkConfirmButton: Locator
	readonly unlinkCancelButton: Locator

	constructor(page: Page) {
		super(page)

		this.createTemplateButton = page.getByRole('button', { name: /create template/i })
		this.searchInput = page.getByPlaceholder(/search by name/i)
		this.table = page.locator('table')
		this.tableRows = page.locator('table tbody tr')
		this.modalDialog = page.locator('[role="dialog"]')
		this.modalDetail = page.locator('[role="dialog"]').filter({ hasText: /read one template/i })
		this.modalDeleteConfirm = page
			.locator('[role="dialog"]')
			.filter({ has: page.getByRole('heading', { name: /^delete$/i }) })
		this.modalUnlinkConfirm = page
			.locator('[role="dialog"]')
			.filter({ hasText: /unlink templates confirmation/i })

		this.formNameInput = page.getByPlaceholder(/names of template/i)
		this.formClusterInput = page.locator('.app-input-cluster [tabindex="0"]')
		this.formLinkTemplatesTrigger = page
			.locator('[role="dialog"]')
			.locator('div.space-y-1\\.5')
			.filter({ has: page.getByText('Link Templates', { exact: true }) })
			.locator('.select-trigger')
			.first()
		this.formSaveButton = page
			.locator('[role="dialog"]')
			.getByRole('button', { name: /^save$/i })
		this.formCancelButton = page
			.locator('[role="dialog"]')
			.getByRole('button', { name: /^cancel$/i })

		this.unlinkConfirmButton = this.modalUnlinkConfirm.getByRole('button', {
			name: /^update$/i
		})
		this.unlinkCancelButton = this.modalUnlinkConfirm.getByRole('button', { name: /^cancel$/i })
	}

	async gotoTemplates() {
		await this.goto('/configuration/template')
		await this.page.waitForLoadState('domcontentloaded')
		await this.createTemplateButton.waitFor({ state: 'visible', timeout: 20_000 })
	}

	async openCreateModal() {
		await this.createTemplateButton.click()
		await this.modalDialog.waitFor({ state: 'visible', timeout: 5_000 })
	}

	async fillTemplateName(name: string) {
		await this.formNameInput.fill(name)
		await this.formNameInput.dispatchEvent('input')
		await this.formNameInput.blur()
	}

	async selectLinkTemplate(templateName: string) {
		await this.formLinkTemplatesTrigger.click()
		await this.clusterDropdownTeleport.waitFor({ state: 'visible', timeout: 5_000 })
		const option = this.clusterOptions.filter({ hasText: templateName }).first()
		await option.scrollIntoViewIfNeeded()
		await option.click()
		// Close dropdown by clicking the trigger again (do not press Escape, as Escape closes parent UiDialog)
		if (await this.clusterDropdownTeleport.isVisible()) {
			await this.formLinkTemplatesTrigger.click()
		}
	}

	async submitForm() {
		await this.formSaveButton.click()
	}

	async searchTemplate(name: string) {
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
							url.pathname.includes('/api/templates') &&
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
							url.pathname.includes('/api/templates') &&
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

	async openDetail(templateName: string) {
		const row = await this.getRowByName(templateName)
		const actionButton = row.locator('button').last()
		await actionButton.click()

		const detailOption = this.page.getByRole('menuitem', { name: /view detail/i })
		await detailOption.click()
		await this.modalDetail.waitFor({ state: 'visible', timeout: 5_000 })
	}

	async openEdit(templateName: string) {
		const row = await this.getRowByName(templateName)
		const actionButton = row.locator('button').last()
		await actionButton.click()

		const editOption = this.page.getByRole('menuitem', { name: /edit template/i })
		await editOption.click()
		await this.modalDialog.waitFor({ state: 'visible', timeout: 5_000 })
	}

	async deleteTemplate(templateName: string) {
		const row = await this.getRowByName(templateName)
		const actionButton = row.locator('button').last()
		await actionButton.click()

		const deleteOption = this.page.getByRole('menuitem', { name: /delete template/i })
		await deleteOption.click()
		await this.modalDeleteConfirm.waitFor({ state: 'visible', timeout: 5_000 })

		const confirmDeleteBtn = this.modalDeleteConfirm.getByRole('button', { name: /^delete$/i })
		await confirmDeleteBtn.click()
		await this.modalDeleteConfirm.waitFor({ state: 'hidden', timeout: 15_000 })
	}

	async clickItemsLink(templateName: string) {
		const row = await this.getRowByName(templateName)
		await row.waitFor({ state: 'visible', timeout: 10_000 })
		const itemsLink = row.getByRole('button', { name: /items/i }).first()
		await itemsLink.click()
		await this.page.waitForLoadState('domcontentloaded')
	}

	async goBackFromItems() {
		const backBtn = this.page
			.locator('div.flex.items-center.gap-3')
			.filter({ has: this.page.getByRole('heading', { name: /template items/i }) })
			.locator('button')
			.first()

		await backBtn.click()
		try {
			await this.page.waitForURL(/\/configuration\/template(\/)?(\?.*)?$/, { timeout: 5_000 })
		} catch {
			await this.gotoTemplates()
		}
	}
}

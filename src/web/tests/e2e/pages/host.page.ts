// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { Locator, Page } from '@playwright/test'
import { BasePage } from './base.page'

export class HostPage extends BasePage {
	readonly createHostButton: Locator
	readonly searchInput: Locator
	readonly nodeFilterSelect: Locator
	readonly table: Locator
	readonly tableRows: Locator
	readonly modalDialog: Locator
	readonly modalDetail: Locator
	readonly modalUnapplyTemplate: Locator
	readonly modalDeleteConfirm: Locator

	// Form fields
	readonly formNameInput: Locator
	readonly formClusterInput: Locator
	readonly formStatusSwitch: Locator
	readonly formAddLabelButton: Locator
	readonly formTemplateSelect: Locator
	readonly formSaveButton: Locator
	readonly formCancelButton: Locator

	constructor(page: Page) {
		super(page)

		this.createHostButton = page.getByRole('button', { name: /create host/i })
		this.searchInput = page.getByPlaceholder(/search by name/i)
		this.nodeFilterSelect = page
			.locator('.select-trigger')
			.filter({ hasText: /select node/i })
			.first()
		this.table = page.locator('table')
		this.tableRows = page.locator('table tbody tr')
		this.modalDialog = page.locator('[role="dialog"]')
		this.modalDetail = page.locator('[role="dialog"]').filter({ hasText: /read one host/i })
		this.modalUnapplyTemplate = page
			.locator('[role="dialog"]')
			.filter({ hasText: /unlink templates confirmation/i })
		this.modalDeleteConfirm = page
			.locator('[role="dialog"]')
			.filter({ hasText: /delete host/i })

		this.formNameInput = page.getByPlaceholder(/name of hosts/i)
		this.formClusterInput = page.locator('.app-input-cluster [tabindex="0"]')
		this.formStatusSwitch = page.locator('[role="dialog"]').getByRole('switch')
		this.formAddLabelButton = page.getByRole('button', { name: /add label/i })
		this.formTemplateSelect = page
			.locator('[role="dialog"]')
			.locator('div:has(> label:has-text("Templates")) .select-trigger')
			.first()
		this.formSaveButton = page
			.locator('[role="dialog"]')
			.getByRole('button', { name: /^save$/i })
		this.formCancelButton = page
			.locator('[role="dialog"]')
			.getByRole('button', { name: /^cancel$/i })
	}

	async gotoHosts() {
		await this.goto('/configuration/host')
		await this.page.waitForSelector('header nav, table', { state: 'visible', timeout: 15_000 })
	}

	async openCreateModal() {
		await this.createHostButton.click()
		await this.modalDialog.waitFor({ state: 'visible', timeout: 10_000 })
	}

	async fillHostName(name: string) {
		await this.formNameInput.fill(name)
	}

	async setStatus(active: boolean) {
		const isChecked = (await this.formStatusSwitch.getAttribute('aria-checked')) === 'true'
		if (isChecked !== active) {
			await this.formStatusSwitch.click()
		}
	}

	async addLabel(key: string, value: string) {
		await this.formAddLabelButton.click()
		const keyInputs = this.modalDialog.getByPlaceholder('Key')
		const valInputs = this.modalDialog.getByPlaceholder('Value')
		const lastIndex = (await keyInputs.count()) - 1
		if (lastIndex >= 0) {
			const keyInput = keyInputs.nth(lastIndex)
			const valInput = valInputs.nth(lastIndex)
			await keyInput.fill(key)
			await keyInput.dispatchEvent('input')
			await valInput.fill(value)
			await valInput.dispatchEvent('input')
			await valInput.blur()
		}
	}

	async selectTemplate(templateName: string) {
		await this.formTemplateSelect.click()
		await this.clusterDropdownTeleport.waitFor({ state: 'visible', timeout: 5_000 })
		const option = this.clusterOptions.filter({ hasText: templateName }).first()
		await option.click()
		// Close dropdown by clicking trigger again (not Escape to avoid closing parent dialog)
		await this.formTemplateSelect.click()
	}

	async getAvailableTemplateCount(): Promise<number> {
		await this.formTemplateSelect.click()
		let count = 0
		try {
			await this.clusterDropdownTeleport.waitFor({ state: 'visible', timeout: 3_000 })
			count = await this.clusterOptions.count()
		} catch {
			count = 0
		}
		await this.formTemplateSelect.click()
		return count
	}

	async selectFirstTemplate(): Promise<string> {
		await this.formTemplateSelect.click()
		await this.clusterDropdownTeleport.waitFor({ state: 'visible', timeout: 5_000 })
		const firstOption = this.clusterOptions.first()
		const templateText = (await firstOption.textContent())?.trim() || ''
		await firstOption.click()
		await this.formTemplateSelect.click()
		return templateText
	}

	async deselectFirstTemplate() {
		await this.formTemplateSelect.click()
		await this.clusterDropdownTeleport.waitFor({ state: 'visible', timeout: 5_000 })
		const firstOption = this.clusterOptions.first()
		await firstOption.click()
		await this.formTemplateSelect.click()
	}

	async submitForm() {
		await this.formSaveButton.click()
	}

	async getRowByName(name: string): Promise<Locator> {
		return this.tableRows.filter({ hasText: name }).first()
	}

	async searchHost(query: string) {
		const currentVal = await this.searchInput.inputValue()
		if (currentVal === query) {
			await this.table.waitFor({ state: 'visible' })
			return
		}

		const searchPromise = this.page
			.waitForResponse(
				(res) => {
					try {
						const url = new URL(res.url())
						return (
							url.pathname.includes('/api/hosts') &&
							url.searchParams.get('name') === query &&
							res.status() < 400
						)
					} catch {
						return false
					}
				},
				{ timeout: 10_000 }
			)
			.catch(() => null)
		await this.searchInput.fill(query)
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
							url.pathname.includes('/api/hosts') &&
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

	async openRowMenu(hostName: string) {
		const row = await this.getRowByName(hostName)
		await row.waitFor({ state: 'visible', timeout: 10_000 })
		const actionMenuBtn = row.locator('button').last()
		await actionMenuBtn.click()
	}

	async openDetail(hostName: string) {
		await this.openRowMenu(hostName)
		const viewDetailItem = this.page.getByRole('menuitem', { name: /view detail/i })
		await viewDetailItem.click()
		await this.modalDetail.waitFor({ state: 'visible', timeout: 10_000 })
	}

	async openEdit(hostName: string) {
		await this.openRowMenu(hostName)
		const editItem = this.page.getByRole('menuitem', { name: /edit host/i })
		await editItem.click()
		await this.modalDialog.waitFor({ state: 'visible', timeout: 10_000 })
	}

	async deleteHost(hostName: string) {
		await this.openRowMenu(hostName)
		const deleteItem = this.page.getByRole('menuitem', { name: /delete host/i })
		await deleteItem.click()
		await this.modalDeleteConfirm.waitFor({ state: 'visible', timeout: 10_000 })
		const confirmBtn = this.modalDeleteConfirm.getByRole('button', { name: /delete/i })
		await confirmBtn.click()
		await this.modalDeleteConfirm.waitFor({ state: 'hidden', timeout: 10_000 })
	}

	async confirmUnapplyTemplate(keepItems: boolean = false) {
		await this.modalUnapplyTemplate.waitFor({ state: 'visible', timeout: 10_000 })
		const radioOption = this.modalUnapplyTemplate.locator(
			`input[type="radio"][value="${keepItems}"]`
		)
		if (await radioOption.isVisible()) {
			await radioOption.check()
		}
		const updateBtn = this.modalUnapplyTemplate.getByRole('button', { name: /^update$/i })
		await updateBtn.click()
		await this.modalUnapplyTemplate.waitFor({ state: 'hidden', timeout: 10_000 })
	}
}

// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { Locator, Page } from '@playwright/test'
import { BasePage } from './base.page'

export class ConfigurationPage extends BasePage {
	readonly createButton: Locator
	readonly table: Locator
	readonly modalDialog: Locator
	readonly formSaveButton: Locator
	readonly formCancelButton: Locator
	readonly deleteConfirmButton: Locator

	constructor(page: Page) {
		super(page)
		this.createButton = page.getByRole('button', { name: /create/i })
		this.table = page.locator('table')
		this.modalDialog = page.locator('[role="dialog"]')
		this.formSaveButton = page
			.locator('[role="dialog"]')
			.getByRole('button', { name: /^save$/i })
		this.formCancelButton = page
			.locator('[role="dialog"]')
			.getByRole('button', { name: /^cancel$/i })
		this.deleteConfirmButton = page
			.locator('[role="dialog"]')
			.getByRole('button', { name: /^delete$/i })
	}

	async gotoEndpoints() {
		await this.goto('/configuration/endpoint')
		await this.page.waitForSelector('h1, h2, h3, header', { state: 'visible' })
	}

	async openCreateModal() {
		await this.createButton.click()
		await this.modalDialog.waitFor({ state: 'visible' })
	}

	async submitForm() {
		await this.formSaveButton.click()
	}

	async getRowByName(name: string): Promise<Locator> {
		return this.page.locator('tbody tr', { hasText: name })
	}

	async deleteRow(name: string) {
		const row = await this.getRowByName(name)
		const actionMenuBtn = row.locator('button').last()
		await actionMenuBtn.click()

		const deleteItem = this.page.getByRole('menuitem', { name: /delete/i })
		await deleteItem.click()
		await this.deleteConfirmButton.waitFor({ state: 'visible' })
		await this.deleteConfirmButton.click()
	}
}

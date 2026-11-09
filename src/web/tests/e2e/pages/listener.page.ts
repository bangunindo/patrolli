// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { Locator, Page } from '@playwright/test'
import { BasePage } from './base.page'

export class ListenerPage extends BasePage {
	readonly createListenerButton: Locator
	readonly searchInput: Locator
	readonly filterDropdownButton: Locator
	readonly table: Locator
	readonly tableRows: Locator
	readonly modalDialog: Locator
	readonly modalDetail: Locator
	readonly modalDeleteConfirm: Locator

	// Form fields
	readonly formNameInput: Locator
	readonly formClusterInput: Locator
	readonly formStatusSwitch: Locator
	readonly formTypeSelect: Locator
	readonly formSaveButton: Locator
	readonly formCancelButton: Locator

	constructor(page: Page) {
		super(page)

		this.createListenerButton = page.getByRole('button', { name: /create listener/i })
		this.searchInput = page.getByPlaceholder(/search by name/i)
		this.filterDropdownButton = page.getByRole('button', { name: /^filter$/i })
		this.table = page.locator('table')
		this.tableRows = page.locator('table tbody tr')
		this.modalDialog = page.locator('[role="dialog"]')
		this.modalDetail = page.locator('[role="dialog"]').filter({ hasText: /read one listener/i })
		this.modalDeleteConfirm = page
			.locator('[role="dialog"]')
			.filter({ hasText: /delete listener/i })

		this.formNameInput = page.getByPlaceholder(/name of listener/i)
		this.formClusterInput = page.locator('.app-input-cluster [tabindex="0"]')
		this.formStatusSwitch = page.locator('[role="dialog"]').getByRole('switch').first()
		this.formTypeSelect = page
			.locator('[role="dialog"]')
			.locator('div:has(> label:has-text("Listener Type")) .select-trigger')
			.first()
		this.formSaveButton = page
			.locator('[role="dialog"]')
			.getByRole('button', { name: /^save$/i })
		this.formCancelButton = page
			.locator('[role="dialog"]')
			.getByRole('button', { name: /^cancel$/i })
	}

	async gotoListeners() {
		await this.goto('/configuration/listener')
		await this.page.waitForLoadState('domcontentloaded')
		await this.createListenerButton.waitFor({ state: 'visible', timeout: 30_000 })
	}

	async openCreateModal() {
		await this.createListenerButton.click()
		await this.modalDialog.waitFor({ state: 'visible', timeout: 5_000 })
	}

	async fillListenerName(name: string) {
		await this.formNameInput.fill(name)
		await this.formNameInput.dispatchEvent('input')
		await this.formNameInput.blur()
	}

	async submitForm() {
		await this.formSaveButton.click()
	}

	async searchListener(name: string) {
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
							url.pathname.includes('/api/listeners') &&
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
							url.pathname.includes('/api/listeners') &&
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

	async openDetail(listenerName: string) {
		const row = await this.getRowByName(listenerName)
		const actionButton = row.locator('button').last()
		await actionButton.click()

		const detailOption = this.page.getByRole('menuitem', { name: /view detail/i })
		await detailOption.click()
		await this.modalDetail.waitFor({ state: 'visible', timeout: 5_000 })
	}

	async openEdit(listenerName: string) {
		const row = await this.getRowByName(listenerName)
		const actionButton = row.locator('button').last()
		await actionButton.click()

		const editOption = this.page.getByRole('menuitem', { name: /edit listener/i })
		await editOption.click()
		await this.modalDialog.waitFor({ state: 'visible', timeout: 5_000 })
	}

	async deleteListener(listenerName: string) {
		const row = await this.getRowByName(listenerName)
		const actionButton = row.locator('button').last()
		await actionButton.click()

		const deleteOption = this.page.getByRole('menuitem', { name: /delete listener/i })
		await deleteOption.click()
		await this.modalDeleteConfirm.waitFor({ state: 'visible', timeout: 5_000 })

		const confirmDeleteBtn = this.modalDeleteConfirm.getByRole('button', { name: /^delete$/i })
		await confirmDeleteBtn.click()
		await this.modalDeleteConfirm.waitFor({ state: 'hidden', timeout: 15_000 })
	}
}

// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { Locator, Page } from '@playwright/test'
import { expect } from '@playwright/test'
import { BasePage } from './base.page'

export class EndpointPage extends BasePage {
	readonly createEndpointButton: Locator
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
	readonly formHostTrigger: Locator
	readonly formStatusSwitch: Locator
	readonly formProtocolSelect: Locator
	readonly formAddressInput: Locator
	readonly formCommunityInput: Locator
	readonly formUrlInput: Locator
	readonly formSaveButton: Locator
	readonly formCancelButton: Locator

	// Select Host Dialog
	readonly selectHostDialog: Locator
	readonly selectHostSearchInput: Locator
	readonly selectHostApplyButton: Locator

	constructor(page: Page) {
		super(page)

		this.createEndpointButton = page.getByRole('button', { name: /create endpoint/i })
		this.searchInput = page.getByPlaceholder(/search by name/i)
		this.filterDropdownButton = page.getByRole('button', { name: /^filter$/i })
		this.table = page.locator('table')
		this.tableRows = page.locator('table tbody tr')
		this.modalDialog = page.locator('[role="dialog"]')
		this.modalDetail = page.locator('[role="dialog"]').filter({ hasText: /read one endpoint/i })
		this.modalDeleteConfirm = page
			.locator('[role="dialog"]')
			.filter({ hasText: /delete endpoint/i })

		this.formNameInput = page.getByPlaceholder(/name of endpoint/i)
		this.formClusterInput = page
			.locator('[role="dialog"]')
			.locator('div:has(> label:has-text("Cluster")) .select-trigger')
			.first()
		this.formHostTrigger = page
			.locator('[role="dialog"]')
			.locator('div:has(> label:has-text("Host"))')
			.locator('.cursor-pointer')
			.first()
		this.formStatusSwitch = page.locator('[role="dialog"]').getByRole('switch')
		this.formProtocolSelect = page
			.locator('[role="dialog"]')
			.locator('div:has(> label:has-text("Protocol")) .select-trigger')
			.first()
		this.formAddressInput = page.getByPlaceholder(/192\.168\.1\.1:161/i)
		this.formCommunityInput = page
			.locator('[role="dialog"]')
			.locator('input[type="password"]')
			.first()
		this.formUrlInput = page.getByPlaceholder(/http:\/\/localhost:8080/i)
		this.formSaveButton = page
			.locator('[role="dialog"]')
			.getByRole('button', { name: /^save$/i })
		this.formCancelButton = page
			.locator('[role="dialog"]')
			.getByRole('button', { name: /^cancel$/i })

		// Host selection modal (has specific heading 'Select Host' inside UiDialogTitle)
		this.selectHostDialog = page
			.locator('[role="dialog"]')
			.filter({ has: page.getByRole('heading', { name: /select host/i }) })
		this.selectHostSearchInput = this.selectHostDialog.getByPlaceholder(/search by host name/i)
		this.selectHostApplyButton = this.selectHostDialog.getByRole('button', { name: /^apply$/i })
	}

	async gotoEndpoints() {
		await this.goto('/configuration/endpoint')
		await this.page.waitForLoadState('domcontentloaded')
		await this.createEndpointButton.waitFor({ state: 'visible', timeout: 30_000 })
	}

	async openCreateModal() {
		await this.createEndpointButton.click()
		await this.modalDialog.waitFor({ state: 'visible', timeout: 5_000 })
		await expect(this.formClusterInput).toContainText(/\w+/, { timeout: 10_000 })
	}

	async fillEndpointName(name: string) {
		await this.formNameInput.fill(name)
		await this.formNameInput.dispatchEvent('input')
		await this.formNameInput.blur()
	}

	async selectHost(hostName: string) {
		await this.formHostTrigger.click()
		await this.selectHostDialog.waitFor({ state: 'visible', timeout: 5_000 })

		if (await this.selectHostSearchInput.isVisible()) {
			const currentVal = await this.selectHostSearchInput.inputValue()
			if (currentVal !== hostName) {
				const searchPromise = this.page
					.waitForResponse(
						(resp) => {
							try {
								const url = new URL(resp.url())
								return (
									url.pathname.includes('/api/hosts') &&
									url.searchParams.get('name') === hostName &&
									resp.status() < 400
								)
							} catch {
								return false
							}
						},
						{ timeout: 10_000 }
					)
					.catch(() => null)
				await this.selectHostSearchInput.fill(hostName)
				await searchPromise
			}
		}

		// Locate the row containing the host name inside SelectTable
		const hostRow = this.selectHostDialog
			.locator('table tbody tr')
			.filter({ hasText: hostName })
			.first()
		await hostRow.waitFor({ state: 'visible', timeout: 5_000 })

		const checkbox = hostRow.locator('button[role="checkbox"]').first()
		await checkbox.scrollIntoViewIfNeeded()
		await checkbox.click()
		await expect(checkbox).toHaveAttribute('data-state', 'checked', { timeout: 5_000 })
		await expect(hostRow).toHaveClass(/bg-primary/, { timeout: 5_000 })

		// Click Apply
		await this.selectHostApplyButton.click()
		await this.selectHostDialog.waitFor({ state: 'hidden', timeout: 5_000 })

		// Assert host is selected on the form
		await expect(this.formHostTrigger).toContainText('( 1 /', { timeout: 10_000 })
	}

	async selectProtocol(protocolName: string) {
		await this.formProtocolSelect.waitFor({ state: 'visible', timeout: 10_000 })
		await this.formProtocolSelect.click()
		const option = this.clusterOptions
			.filter({ hasText: new RegExp(`^${protocolName}$`, 'i') })
			.first()
		await option.waitFor({ state: 'visible', timeout: 10_000 })
		await option.click()
	}

	async fillSnmpV1OrV2c(address: string, community: string) {
		if (address) {
			await this.formAddressInput.fill(address)
			await this.formAddressInput.dispatchEvent('input')
			await this.formAddressInput.blur()
		}
		if (community) {
			await this.formCommunityInput.fill(community)
			await this.formCommunityInput.dispatchEvent('input')
			await this.formCommunityInput.blur()
		}
	}

	async fillHttpUrl(url: string) {
		await this.formUrlInput.fill(url)
		await this.formUrlInput.dispatchEvent('input')
		await this.formUrlInput.blur()
	}

	async submitForm() {
		await this.formSaveButton.click()
	}

	async searchEndpoint(name: string) {
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
							url.pathname.includes('/api/endpoints') &&
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
							url.pathname.includes('/api/endpoints') &&
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

	async openDetail(endpointName: string) {
		const row = await this.getRowByName(endpointName)
		const actionButton = row.locator('button').last()
		await actionButton.click()

		const detailOption = this.page.getByRole('menuitem', { name: /view detail/i })
		await detailOption.click()
		await this.modalDetail.waitFor({ state: 'visible', timeout: 5_000 })
	}

	async openEdit(endpointName: string) {
		const row = await this.getRowByName(endpointName)
		const actionButton = row.locator('button').last()
		await actionButton.click()

		const editOption = this.page.getByRole('menuitem', { name: /edit endpoint/i })
		await editOption.click()
		await this.modalDialog.waitFor({ state: 'visible', timeout: 5_000 })
	}

	async deleteEndpoint(endpointName: string) {
		const row = await this.getRowByName(endpointName)
		const actionButton = row.locator('button').last()
		await actionButton.click()

		const deleteOption = this.page.getByRole('menuitem', { name: /delete endpoint/i })
		await deleteOption.click()
		await this.modalDeleteConfirm.waitFor({ state: 'visible', timeout: 5_000 })

		const confirmDeleteBtn = this.modalDeleteConfirm.getByRole('button', { name: /^delete$/i })
		await confirmDeleteBtn.click()
		await this.modalDeleteConfirm.waitFor({ state: 'hidden', timeout: 15_000 })
	}
}

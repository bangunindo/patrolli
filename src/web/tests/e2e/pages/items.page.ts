// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { Locator, Page } from '@playwright/test'
import { expect } from '@playwright/test'
import { BasePage } from './base.page'

export class ItemsPage extends BasePage {
	readonly createItemButton: Locator
	readonly searchInput: Locator
	readonly filterDropdownButton: Locator
	readonly table: Locator
	readonly tableRows: Locator
	readonly modalDialog: Locator
	readonly modalDetail: Locator
	readonly modalDeleteConfirm: Locator
	readonly drawer: Locator
	readonly drawerCloseButton: Locator

	// Form fields
	readonly formNameInput: Locator
	readonly formHostTrigger: Locator
	readonly formEndpointTrigger: Locator
	readonly formScheduleInput: Locator
	readonly formTimeoutInput: Locator
	readonly formOidInput: Locator
	readonly formSaveButton: Locator
	readonly formCancelButton: Locator

	// Select Host Dialog
	readonly selectHostDialog: Locator
	readonly selectHostSearchInput: Locator
	readonly selectHostApplyButton: Locator

	// Select Endpoint Dialog
	readonly selectEndpointDialog: Locator
	readonly selectEndpointSearchInput: Locator
	readonly selectEndpointApplyButton: Locator

	constructor(page: Page) {
		super(page)

		this.createItemButton = page.getByRole('button', { name: /create item for host/i })
		this.searchInput = page.getByPlaceholder(/search by name/i)
		this.filterDropdownButton = page.getByRole('button', { name: /^filter$/i })
		this.table = page.locator('table')
		this.tableRows = page.locator('table tbody tr')
		this.modalDialog = page.locator('[role="dialog"]')
		this.modalDetail = page.locator('[role="dialog"]').filter({ hasText: /read one item/i })
		this.modalDeleteConfirm = page
			.locator('[role="dialog"]')
			.filter({ has: page.getByRole('heading', { name: /^delete$/i }) })

		this.drawer = page.locator('aside[aria-label="Item Statistics"]')
		this.drawerCloseButton = this.drawer.getByRole('button', { name: /^close$/i }).last()

		this.formNameInput = page
			.locator('#item-form')
			.locator('div.space-y-1\\.5')
			.filter({ has: page.locator('label:has-text("Name")') })
			.locator('input')
		this.formHostTrigger = page
			.locator('#item-form')
			.locator('div.space-y-1\\.5')
			.filter({ has: page.locator('label:has-text("Host")') })
			.locator('.cursor-pointer')
			.first()
		this.formEndpointTrigger = page
			.locator('#item-form')
			.locator('div.space-y-1\\.5')
			.filter({ has: page.locator('label:has-text("Endpoint")') })
			.locator('.cursor-pointer')
			.first()
		this.formScheduleInput = page
			.locator('#item-form')
			.locator('div.space-y-1\\.5')
			.filter({ has: page.locator('label:has-text("Schedule")') })
			.locator('input')
		this.formTimeoutInput = page
			.locator('#item-form')
			.locator('div.space-y-1\\.5')
			.filter({ has: page.locator('label:has-text("Timeout")') })
			.locator('input')
		this.formOidInput = page
			.locator('#item-form')
			.locator('div.space-y-1\\.5')
			.filter({ has: page.locator('label:has-text("OID")') })
			.locator('input')
		this.formSaveButton = page.locator('#item-form').getByRole('button', { name: /^save$/i })
		this.formCancelButton = page
			.locator('#item-form')
			.getByRole('button', { name: /^cancel$/i })

		this.selectHostDialog = page
			.locator('[role="dialog"]')
			.filter({ has: page.getByRole('heading', { name: /select host/i }) })
		this.selectHostSearchInput = this.selectHostDialog.getByPlaceholder(/search by host name/i)
		this.selectHostApplyButton = this.selectHostDialog.getByRole('button', { name: /^apply$/i })

		this.selectEndpointDialog = page
			.locator('[role="dialog"]')
			.filter({ has: page.getByRole('heading', { name: /select endpoint/i }) })
		this.selectEndpointSearchInput =
			this.selectEndpointDialog.getByPlaceholder(/search by endpoint name/i)
		this.selectEndpointApplyButton = this.selectEndpointDialog.getByRole('button', {
			name: /^apply$/i
		})
	}

	async gotoItems(clusterName: string = 'patrolli') {
		await this.goto(`/configuration/item?cluster=${clusterName}`)
		await this.page.waitForLoadState('domcontentloaded')
		await this.createItemButton.waitFor({ state: 'visible', timeout: 30_000 })
	}

	async openCreateModal() {
		await this.createItemButton.click()
		await this.modalDialog.waitFor({ state: 'visible', timeout: 5_000 })
	}

	async selectHost(hostName: string) {
		await this.formHostTrigger.click()
		await this.selectHostDialog.waitFor({ state: 'visible', timeout: 10_000 })

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

		const hostRow = this.selectHostDialog
			.locator('table tbody tr')
			.filter({ hasText: hostName })
			.first()
		await hostRow.waitFor({ state: 'visible', timeout: 10_000 })

		const checkbox = hostRow.locator('button[role="checkbox"]').first()
		await checkbox.scrollIntoViewIfNeeded()
		await checkbox.click()
		await expect(checkbox).toHaveAttribute('data-state', 'checked', { timeout: 5_000 })
		await expect(hostRow).toHaveClass(/bg-primary/, { timeout: 5_000 })

		await this.selectHostApplyButton.click()
		await this.selectHostDialog.waitFor({ state: 'hidden', timeout: 10_000 })
		await expect(this.formHostTrigger).toContainText('( 1 /', { timeout: 10_000 })
	}

	async selectEndpoint(endpointName: string) {
		await this.formEndpointTrigger.click()
		await this.selectEndpointDialog.waitFor({ state: 'visible', timeout: 10_000 })

		if (await this.selectEndpointSearchInput.isVisible()) {
			const currentVal = await this.selectEndpointSearchInput.inputValue()
			if (currentVal !== endpointName) {
				const searchPromise = this.page
					.waitForResponse(
						(resp) => {
							try {
								const url = new URL(resp.url())
								return (
									url.pathname.includes('/api/endpoints') &&
									url.searchParams.get('name') === endpointName &&
									resp.status() < 400
								)
							} catch {
								return false
							}
						},
						{ timeout: 10_000 }
					)
					.catch(() => null)
				await this.selectEndpointSearchInput.fill(endpointName)
				await searchPromise
			}
		}

		const epRow = this.selectEndpointDialog
			.locator('table tbody tr')
			.filter({ hasText: endpointName })
			.first()
		await epRow.waitFor({ state: 'visible', timeout: 10_000 })

		const checkbox = epRow.locator('button[role="checkbox"]').first()
		await checkbox.scrollIntoViewIfNeeded()
		await checkbox.click()
		await expect(checkbox).toHaveAttribute('data-state', 'checked', { timeout: 5_000 })
		await expect(epRow).toHaveClass(/bg-primary/, { timeout: 5_000 })

		await this.selectEndpointApplyButton.click()
		await this.selectEndpointDialog.waitFor({ state: 'hidden', timeout: 10_000 })
		await expect(this.formEndpointTrigger).toContainText('( 1 /', { timeout: 10_000 })
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

	async cancelForm() {
		await this.formCancelButton.click()
		await this.modalDialog.waitFor({ state: 'hidden', timeout: 5_000 })
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
		await row.waitFor({ state: 'visible', timeout: 10_000 })
		const actionButton = row.locator('button').last()
		await actionButton.click()

		const detailOption = this.page.getByRole('menuitem', { name: /view detail/i })
		await detailOption.waitFor({ state: 'visible', timeout: 5_000 })
		await detailOption.click()
		await this.modalDetail.waitFor({ state: 'visible', timeout: 5_000 })
	}

	async openTrends(itemName: string) {
		const row = await this.getRowByName(itemName)
		await row.waitFor({ state: 'visible', timeout: 10_000 })
		const actionButton = row.locator('button').last()
		await actionButton.click()

		const trendsOption = this.page.getByRole('menuitem', { name: /view trends/i })
		await trendsOption.waitFor({ state: 'visible', timeout: 5_000 })
		const itemFetchPromise = this.page.waitForResponse(
			(res) => res.url().includes('/api/items/') && res.status() === 200,
			{ timeout: 10_000 }
		)
		await trendsOption.click()
		await itemFetchPromise
		await this.drawer.waitFor({ state: 'visible', timeout: 5_000 })
	}

	async closeDrawer() {
		await this.drawerCloseButton.click()
		await this.drawer.waitFor({ state: 'hidden', timeout: 5_000 })
	}

	async openEdit(itemName: string) {
		const row = await this.getRowByName(itemName)
		await row.waitFor({ state: 'visible', timeout: 10_000 })
		const actionButton = row.locator('button').last()
		await actionButton.click()

		const editOption = this.page.getByRole('menuitem', { name: /edit item/i })
		await editOption.waitFor({ state: 'visible', timeout: 5_000 })
		await editOption.click()
		await this.modalDialog.waitFor({ state: 'visible', timeout: 5_000 })
	}

	async deleteItem(itemName: string) {
		const row = await this.getRowByName(itemName)
		await row.waitFor({ state: 'visible', timeout: 10_000 })
		const actionButton = row.locator('button').last()
		await actionButton.click()

		const deleteOption = this.page.getByRole('menuitem', { name: /delete item/i })
		await deleteOption.waitFor({ state: 'visible', timeout: 5_000 })
		await deleteOption.click()
		await this.modalDeleteConfirm.waitFor({ state: 'visible', timeout: 5_000 })

		const confirmDeleteBtn = this.modalDeleteConfirm.getByRole('button', { name: /^delete$/i })
		await confirmDeleteBtn.click()
		await this.modalDeleteConfirm.waitFor({ state: 'hidden', timeout: 15_000 })
	}
}

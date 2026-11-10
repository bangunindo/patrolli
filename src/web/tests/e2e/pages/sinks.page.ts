// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { expect, type Locator, type Page } from '@playwright/test'
import { BasePage } from './base.page'

export class SinkPage extends BasePage {
	readonly createSinkButton: Locator
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
	readonly formWorkerCountInput: Locator
	readonly formBatchSizeInput: Locator
	readonly formBatchTimeoutInput: Locator
	readonly formChannelTypeTrigger: Locator
	readonly formConfigTypeTrigger: Locator
	readonly formFilePathInput: Locator
	readonly formSaveButton: Locator
	readonly formCancelButton: Locator

	// Kafka specific form fields
	readonly formKafkaHostsInput: Locator
	readonly formKafkaTopicInput: Locator
	readonly formKafkaSecurityTrigger: Locator

	// Delete Modal fields
	readonly deleteConfirmButton: Locator
	readonly deleteCancelButton: Locator

	constructor(page: Page) {
		super(page)

		this.createSinkButton = page.getByRole('button', { name: /create sink/i })
		this.searchInput = page.getByPlaceholder(/search by name/i)
		this.filterDropdownButton = page.getByRole('button', { name: /^filter$/i })
		this.table = page.locator('table')
		this.tableRows = page.locator('table tbody tr')
		this.modalDialog = page.locator('[role="dialog"]')
		this.modalDetail = page.locator('[role="dialog"]').filter({ hasText: /read one sink/i })
		this.modalDeleteConfirm = page
			.locator('[role="dialog"]')
			.filter({ has: page.getByRole('heading', { name: /^delete$/i }) })

		// Form fields inside #sink-form
		this.formNameInput = page.getByPlaceholder(/name of group/i)

		this.formClusterInput = page
			.locator('#sink-form')
			.locator('div.space-y-1\\.5')
			.filter({ has: page.locator('label:has-text("Cluster")') })
			.locator('[tabindex], .cursor-pointer, .cursor-not-allowed')
			.first()

		this.formStatusSwitch = page
			.locator('#sink-form')
			.locator('div')
			.filter({ has: page.locator('label:has-text("Status")') })
			.getByRole('switch')
			.first()

		this.formWorkerCountInput = page
			.locator('#sink-form')
			.getByPlaceholder(/worker count/i)
			.first()

		this.formBatchSizeInput = page
			.locator('#sink-form')
			.getByPlaceholder(/batch size/i)
			.first()

		this.formBatchTimeoutInput = page
			.locator('#sink-form')
			.getByPlaceholder(/batch timeout/i)
			.first()

		this.formChannelTypeTrigger = page
			.locator('#sink-form')
			.locator('div.space-y-1\\.5')
			.filter({ has: page.locator('label:has-text("Chanel Type")') })
			.locator('.select-trigger, button[role="combobox"]')
			.first()

		this.formConfigTypeTrigger = page
			.locator('#sink-form')
			.locator('div.space-y-1\\.5')
			.filter({ has: page.locator('label:has-text("Config Type")') })
			.locator('.select-trigger, button[role="combobox"]')
			.first()

		this.formFilePathInput = page
			.locator('#sink-form')
			.locator('div.space-y-1\\.5')
			.filter({ has: page.locator('label:has-text("Path")') })
			.locator('input')
			.first()

		this.formSaveButton = page.locator('#sink-form').getByRole('button', { name: /^save$/i })

		this.formCancelButton = page
			.locator('#sink-form')
			.getByRole('button', { name: /^cancel$/i })

		// Kafka fields
		this.formKafkaHostsInput = page
			.locator('#sink-form')
			.getByPlaceholder(/input host/i)
			.first()

		this.formKafkaTopicInput = page
			.locator('#sink-form')
			.getByPlaceholder(/input topic/i)
			.first()

		this.formKafkaSecurityTrigger = page
			.locator('#sink-form')
			.locator('div.space-y-1\\.5')
			.filter({ has: page.locator('label:has-text("Security Protocol")') })
			.locator('.select-trigger, button[role="combobox"]')
			.first()

		this.deleteConfirmButton = this.modalDeleteConfirm.getByRole('button', {
			name: /^delete$/i
		})
		this.deleteCancelButton = this.modalDeleteConfirm.getByRole('button', {
			name: /^cancel$/i
		})
	}

	async gotoSinks() {
		await this.goto('/configuration/sink')
		await this.page.waitForLoadState('domcontentloaded')
		await this.createSinkButton.waitFor({ state: 'visible', timeout: 30_000 })
	}

	async openCreateModal() {
		await this.createSinkButton.click()
		await this.modalDialog.waitFor({ state: 'visible', timeout: 5_000 })
	}

	async fillSinkName(name: string) {
		await this.formNameInput.fill(name)
		await this.formNameInput.dispatchEvent('input')
		await this.formNameInput.blur()
	}

	async selectConfigType(typeName: 'file' | 'kafka' | 'open_telemetry' | 'clickhouse') {
		await this.formConfigTypeTrigger.click()
		const option = this.clusterOptions
			.filter({ hasText: new RegExp(typeName.replace('_', ' '), 'i') })
			.first()
		await option.waitFor({ state: 'visible', timeout: 5_000 })
		await option.click()
	}

	async selectKafkaSecurityProtocol(
		protocol: 'plaintext' | 'ssl' | 'sasl_plaintext' | 'sasl_ssl'
	) {
		await this.formKafkaSecurityTrigger.click()
		const option = this.clusterOptions
			.filter({ hasText: new RegExp(protocol.replace('_', ' '), 'i') })
			.first()
		await option.waitFor({ state: 'visible', timeout: 5_000 })
		await option.click()
	}

	async fillFilePath(path: string) {
		await this.formFilePathInput.fill(path)
		await this.formFilePathInput.dispatchEvent('input')
		await this.formFilePathInput.blur()
	}

	async submitForm() {
		const resPromise = this.page
			.waitForResponse(
				(res) => res.url().includes('/api/sinks') && res.request().method() !== 'GET',
				{ timeout: 10_000 }
			)
			.catch(() => null)
		await this.formSaveButton.click()
		await resPromise
	}

	async cancelForm() {
		await this.formCancelButton.click()
	}

	async searchSink(name: string) {
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
							url.pathname.includes('/api/sinks') &&
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
							url.pathname.includes('/api/sinks') &&
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

	async openRowMenu(name: string) {
		const row = await this.getRowByName(name)
		const actionButton = row.locator('button').last()
		await actionButton.click()
		await this.page.locator('[role="menu"]').waitFor({ state: 'visible', timeout: 5_000 })
	}

	async openDetail(name: string) {
		await this.openRowMenu(name)
		const detailMenuItem = this.page.getByRole('menuitem', { name: /view detail/i })
		await detailMenuItem.click()
		await this.modalDetail.waitFor({ state: 'visible', timeout: 5_000 })
	}

	async closeDetail() {
		const closeBtn = this.modalDetail.getByRole('button', { name: /^close$/i }).first()
		await closeBtn.click()
		await this.modalDetail.waitFor({ state: 'hidden', timeout: 5_000 })
	}

	async openEdit(name: string) {
		await this.openRowMenu(name)
		const editMenuItem = this.page.getByRole('menuitem', { name: /edit sink/i })
		const fetchDetailPromise = this.page
			.waitForResponse(
				(res) =>
					res.url().includes('/api/sinks/') &&
					res.request().method() === 'GET' &&
					res.status() < 400,
				{ timeout: 10_000 }
			)
			.catch(() => null)
		await editMenuItem.click()
		await this.modalDialog.waitFor({ state: 'visible', timeout: 5_000 })
		await fetchDetailPromise
		await expect(this.formNameInput).toHaveValue(name, { timeout: 10_000 })
	}

	async deleteSink(name: string) {
		await this.openRowMenu(name)
		const deleteMenuItem = this.page.getByRole('menuitem', { name: /delete sink/i })
		await deleteMenuItem.click()
		await this.modalDeleteConfirm.waitFor({ state: 'visible', timeout: 5_000 })
		await this.deleteConfirmButton.click()
		await this.modalDeleteConfirm.waitFor({ state: 'hidden', timeout: 10_000 })
	}
}

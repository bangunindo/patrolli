// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { Locator, Page } from '@playwright/test'
import { BasePage } from './base.page'

export class PipelinePage extends BasePage {
	readonly heading: Locator
	readonly createPipelineButton: Locator
	readonly searchInput: Locator

	// Table
	readonly table: Locator
	readonly tableRows: Locator

	// Modal Form
	readonly modalDialog: Locator
	readonly formTitle: Locator
	readonly formClusterInput: Locator
	readonly formNameInput: Locator
	readonly formAccessPublicRadio: Locator
	readonly formAccessPrivateRadio: Locator
	readonly buttonSelectHosts: Locator
	readonly hostSelectorPopover: Locator
	readonly buttonSelectAllHosts: Locator
	readonly buttonApplyHosts: Locator
	readonly buttonSelectSinks: Locator
	readonly sinkSelectorPopover: Locator
	readonly buttonApplySinks: Locator
	readonly formSaveButton: Locator
	readonly formCancelButton: Locator

	// Status Modal
	readonly modalStatus: Locator
	readonly modalStatusTitle: Locator
	readonly modalStatusConfirmButton: Locator
	readonly modalStatusCancelButton: Locator

	// Delete Modal
	readonly modalDelete: Locator
	readonly modalDeleteTitle: Locator
	readonly modalDeleteConfirmButton: Locator
	readonly modalDeleteCancelButton: Locator

	constructor(page: Page) {
		super(page)

		this.heading = page.getByRole('heading', { name: /^pipeline management$/i })
		this.createPipelineButton = page.getByRole('button', { name: /create pipeline/i })
		this.searchInput = page.getByPlaceholder(/search pipeline by name/i)

		this.table = page.locator('table')
		this.tableRows = page.locator('table tbody tr')

		// Modal Form
		this.modalDialog = page.locator('[role="dialog"]').filter({
			has: page.getByRole('heading', { name: /create new pipeline|edit pipeline/i })
		})
		this.formTitle = this.modalDialog.getByRole('heading', {
			name: /create new pipeline|edit pipeline/i
		})
		this.formClusterInput = this.modalDialog.locator('.app-input-cluster [tabindex="0"]')
		this.formNameInput = this.modalDialog.getByPlaceholder(/enter pipeline name/i)
		this.formAccessPublicRadio = this.modalDialog
			.locator('div.cursor-pointer')
			.filter({ hasText: /^public/i })
			.or(this.modalDialog.getByText('Public', { exact: true }))
			.first()
		this.formAccessPrivateRadio = this.modalDialog
			.locator('div.cursor-pointer')
			.filter({ hasText: /^private/i })
			.or(this.modalDialog.getByText('Private', { exact: true }))
			.first()
		this.buttonSelectHosts = this.modalDialog.getByRole('button', { name: /select hosts/i })
		this.hostSelectorPopover = this.modalDialog
			.locator('div')
			.filter({ hasText: /select hosts/i })
			.last()
		this.buttonSelectAllHosts = this.modalDialog.getByRole('button', { name: /^select all$/i })
		this.buttonApplyHosts = this.modalDialog.getByRole('button', { name: /apply selection/i })

		this.buttonSelectSinks = this.modalDialog.getByRole('button', { name: /select sinks/i })
		this.sinkSelectorPopover = this.modalDialog
			.locator('div')
			.filter({ hasText: /select sinks/i })
			.last()
		this.buttonApplySinks = this.modalDialog.getByRole('button', { name: /apply selection/i })

		this.formSaveButton = this.modalDialog.getByRole('button', {
			name: /create pipeline|save changes|update pipeline/i
		})
		this.formCancelButton = this.modalDialog.getByRole('button', { name: /cancel/i })

		// Status Modal
		this.modalStatus = page.locator('[role="dialog"]').filter({
			has: page.getByRole('heading', { name: /start pipeline|stop pipeline/i })
		})
		this.modalStatusTitle = this.modalStatus.getByRole('heading', {
			name: /start pipeline|stop pipeline/i
		})
		this.modalStatusConfirmButton = this.modalStatus
			.locator('button')
			.filter({ hasText: /^start$|^stop$/i })
		this.modalStatusCancelButton = this.modalStatus.getByRole('button', { name: /^cancel$/i })

		// Delete Modal
		this.modalDelete = page.locator('[role="dialog"]').filter({
			has: page.getByRole('heading', { name: /delete pipeline/i })
		})
		this.modalDeleteTitle = this.modalDelete.getByRole('heading', { name: /delete pipeline/i })
		this.modalDeleteConfirmButton = this.modalDelete.getByRole('button', { name: /^delete$/i })
		this.modalDeleteCancelButton = this.modalDelete.getByRole('button', { name: /^cancel$/i })
	}

	async gotoPipelineList(): Promise<void> {
		await this.goto('/pipeline')
		await this.page.waitForLoadState('domcontentloaded')
		await this.heading.waitFor({ state: 'visible', timeout: 20000 })
	}

	async navigateFromNavbar(): Promise<void> {
		const isMobile = (this.page.viewportSize()?.width ?? 1024) < 768
		if (isMobile) {
			const mobileMenuButton = this.page.locator('nav .md\\:hidden button').first()
			await mobileMenuButton.click()
			const drawerLink = this.page
				.locator('.md\\:hidden')
				.getByRole('link', { name: /^pipeline$/i })
			await drawerLink.waitFor({ state: 'visible', timeout: 5000 })
			await drawerLink.click()
		} else {
			const desktopLink = this.page.locator('nav').getByRole('link', { name: /^pipeline$/i })
			await desktopLink.click()
		}
		await this.page.waitForURL(/\/pipeline/, { timeout: 10000 })
		await this.heading.waitFor({ state: 'visible', timeout: 15000 })
	}

	getPipelineRow(name: string): Locator {
		return this.tableRows.filter({ hasText: name }).first()
	}

	async searchPipeline(query: string): Promise<void> {
		const searchPromise = this.page
			.waitForResponse((res) => res.url().includes('/api/pipelines') && res.status() < 400, {
				timeout: 7000
			})
			.catch(() => null)
		await this.searchInput.fill(query)
		await searchPromise
		await this.table.waitFor({ state: 'visible' })
	}

	async openPipelineAction(
		name: string,
		action: 'Edit' | 'Open in Builder' | 'Start' | 'Stop' | 'Delete'
	): Promise<void> {
		const row = this.getPipelineRow(name)
		const actionTrigger = row.locator('button').last()
		await actionTrigger.click()
		await this.page.waitForSelector('[role="menu"]', { state: 'visible' })

		const item = this.page.locator('[role="menuitem"]').filter({
			hasText: new RegExp(action, 'i')
		})
		await item.click()
	}

	async fillCreatePipeline(data: { name: string; access?: 'public' | 'private' }): Promise<void> {
		await this.formNameInput.fill(data.name)

		if (data.access === 'private') {
			await this.formAccessPrivateRadio.click()
		} else if (data.access === 'public') {
			await this.formAccessPublicRadio.click()
		}

		// Select Hosts
		await this.buttonSelectHosts.click()
		await this.modalDialog
			.locator('.max-h-\\[220px\\], button[role="checkbox"]')
			.first()
			.waitFor({ state: 'visible', timeout: 5000 })
			.catch(() => null)

		// Check if Select All or first host item is available
		if (await this.buttonSelectAllHosts.isVisible()) {
			await this.buttonSelectAllHosts.click()
		} else {
			const hostItem = this.modalDialog.locator('.max-h-\\[220px\\] > div').first()
			if (await hostItem.isVisible()) {
				await hostItem.click()
			}
		}
		await this.buttonApplyHosts.click()

		// Optional: Select Sinks if available
		try {
			await this.buttonSelectSinks.click()
			const sinkItem = this.modalDialog.locator('.max-h-\\[220px\\] > div').first()
			await sinkItem.waitFor({ state: 'visible', timeout: 3000 })
			await sinkItem.click()
			await this.buttonApplySinks.click()
		} catch {
			// Sinks optional
		}
	}
}

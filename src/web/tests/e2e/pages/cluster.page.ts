// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { Locator, Page } from '@playwright/test'
import { BasePage } from './base.page'

export class ClusterPage extends BasePage {
	readonly pageHeader: Locator
	readonly pageDescription: Locator
	readonly refreshButton: Locator
	readonly table: Locator
	readonly tableHeaders: Locator
	readonly tableRows: Locator
	readonly navbarClusterLink: Locator

	constructor(page: Page) {
		super(page)

		this.pageHeader = page.getByRole('heading', { name: /^clusters$/i })
		this.pageDescription = page.getByText(/overview of all registered nodes in the cluster/i)
		this.refreshButton = page.getByRole('button', { name: /^refresh$/i })
		this.table = page.locator('table')
		this.tableHeaders = page.locator('table thead th')
		this.tableRows = page.locator('table tbody tr')
		this.navbarClusterLink = page
			.locator('header nav')
			.getByRole('link', { name: /cluster monitoring/i })
	}

	async gotoCluster() {
		await this.goto('/cluster')
		await this.page.waitForLoadState('domcontentloaded')
		await this.pageHeader.waitFor({ state: 'visible', timeout: 20_000 })
	}

	async clickRefresh() {
		const responsePromise = this.page.waitForResponse(
			(resp) => resp.url().includes('/api/nodes') && resp.status() === 200,
			{ timeout: 10_000 }
		)
		await this.refreshButton.click()
		await responsePromise
	}

	getNodeRowByName(name: string): Locator {
		return this.tableRows.filter({ hasText: name }).first()
	}

	async navigateFromNavbar() {
		const isMobile = (this.page.viewportSize()?.width ?? 1024) < 768
		if (isMobile) {
			const hamburger = this.page.locator('nav .md\\:hidden button, nav button.h-8').first()
			await hamburger.click()
			const mobilePanel = this.page.locator('.md\\:hidden.bg-card')
			// If not yet visible, try dispatching click directly
			if (!(await mobilePanel.isVisible())) {
				await hamburger.dispatchEvent('click')
			}
			await mobilePanel.waitFor({ state: 'visible', timeout: 5_000 })
			const mobileLink = mobilePanel
				.locator('a')
				.filter({ hasText: /cluster monitoring/i })
				.first()
			await mobileLink.click()
		} else {
			const desktopLink = this.page
				.getByRole('navigation')
				.getByRole('link', { name: /cluster monitoring/i })
				.first()
			await desktopLink.click()
		}
		await this.page.waitForURL((url) => url.pathname === '/cluster', { timeout: 10_000 })
	}
}

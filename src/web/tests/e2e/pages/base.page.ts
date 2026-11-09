// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { expect, type Locator, type Page } from '@playwright/test'

export class BasePage {
	readonly page: Page
	readonly navbar: Locator
	readonly userAvatar: Locator
	readonly logoutButton: Locator
	readonly mobileMenuButton: Locator
	readonly mobileMenuPanel: Locator
	readonly clusterSelectorTrigger: Locator
	readonly clusterDropdownTeleport: Locator
	readonly clusterOptions: Locator

	constructor(page: Page) {
		this.page = page
		this.navbar = page.locator('header nav, nav')
		this.userAvatar = page
			.locator('header, nav')
			.locator('[role="button"], .cursor-pointer')
			.filter({ has: page.locator('.h-7.w-7, [class*="avatar"]') })
		this.logoutButton = page.getByRole('menuitem', { name: /log out/i })
		this.mobileMenuButton = page
			.locator('.md\\:hidden button.h-8.w-8, nav .md\\:hidden button')
			.first()
		this.mobileMenuPanel = page.locator('.md\\:hidden.bg-card, nav + div')
		this.clusterSelectorTrigger = page
			.locator('header, nav')
			.locator('.hidden.md\\:flex [tabindex="0"]')
			.first()
		this.clusterDropdownTeleport = page.locator('.select-dropdown-teleported')
		this.clusterOptions = page.locator('.select-dropdown-teleported .select-option')
	}

	async goto(path: string) {
		await this.page.goto(path, { waitUntil: 'domcontentloaded' })
	}

	async openMobileMenuIfNeeded() {
		if (await this.mobileMenuButton.isVisible()) {
			if (!(await this.mobileMenuPanel.isVisible())) {
				await this.mobileMenuButton.click()
				await this.mobileMenuPanel.waitFor({ state: 'visible', timeout: 5_000 })
			}
		}
	}

	async getActiveClusterTrigger(): Promise<Locator> {
		if (await this.mobileMenuButton.isVisible()) {
			await this.openMobileMenuIfNeeded()
			return this.mobileMenuPanel.locator('[tabindex="0"]').first()
		}
		return this.clusterSelectorTrigger
	}

	async waitForToast(message?: string | RegExp) {
		const toast = this.page.locator('[data-sonner-toast]')
		await toast.waitFor({ state: 'visible', timeout: 10_000 })
		if (message) {
			await this.page.locator('[data-sonner-toast]', { hasText: message }).waitFor({
				state: 'visible',
				timeout: 5_000
			})
		}
		return toast
	}

	async logout() {
		if (await this.mobileMenuButton.isVisible()) {
			await this.openMobileMenuIfNeeded()
			await this.mobileMenuPanel.getByRole('button', { name: /log out/i }).click()
		} else {
			await this.userAvatar.first().click()
			await this.logoutButton.click()
		}
		await this.page.waitForURL((url) => url.pathname.includes('/account/login'), {
			timeout: 10_000
		})
	}

	async expectSingleClusterActive(expectedClusterName?: string) {
		const trigger = await this.getActiveClusterTrigger()
		await trigger.waitFor({ state: 'visible', timeout: 10_000 })
		// In single cluster mode (local form login), the selector is locked and disabled
		await expect(trigger).toHaveClass(/cursor-not-allowed/)
		await expect(trigger).toHaveClass(/opacity-50/)

		if (expectedClusterName) {
			await expect(trigger).toContainText(expectedClusterName)
		}

		// Clicking should NOT open options dropdown
		await trigger.click({ force: true })
		await expect(this.clusterDropdownTeleport).not.toBeVisible()
	}

	async expectMultiClusterActive() {
		const trigger = await this.getActiveClusterTrigger()
		await trigger.waitFor({ state: 'visible', timeout: 10_000 })
		// In multi-cluster mode (Keycloak SSO), the selector is enabled and interactive
		await expect(trigger).not.toHaveClass(/cursor-not-allowed/)

		// Clicking opens options dropdown and reveals cluster options
		await trigger.click()
		await expect(this.clusterDropdownTeleport).toBeVisible({ timeout: 5_000 })
		const count = await this.clusterOptions.count()
		expect(count).toBeGreaterThan(0)

		// Close dropdown by clicking again
		await trigger.click()
	}

	async selectCluster(clusterName: string) {
		const trigger = await this.getActiveClusterTrigger()
		await trigger.waitFor({ state: 'visible', timeout: 10_000 })
		await trigger.click()
		await this.clusterDropdownTeleport.waitFor({ state: 'visible', timeout: 5_000 })

		const option = this.clusterOptions.filter({ hasText: clusterName })
		await option.first().click()

		await expect(trigger).toContainText(clusterName)
	}

	async getSelectedCluster(): Promise<string> {
		const trigger = await this.getActiveClusterTrigger()
		return (await trigger.textContent()) || ''
	}
}

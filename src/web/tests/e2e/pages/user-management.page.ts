// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { Locator, Page } from '@playwright/test'
import { BasePage } from './base.page'

export class UserManagementPage extends BasePage {
	readonly heading: Locator
	readonly refreshButton: Locator

	// Summary Cards
	readonly cardAllowSso: Locator
	readonly cardAllowSsoCount: Locator
	readonly cardDisabled: Locator
	readonly cardDisabledCount: Locator
	readonly cardClusterModule: Locator

	// Toolbar
	readonly searchInput: Locator
	readonly createUserButton: Locator

	// Table
	readonly table: Locator
	readonly tableRows: Locator

	// Modal User Form
	readonly modalForm: Locator
	readonly formTitle: Locator
	readonly formFullNameInput: Locator
	readonly formUsernameInput: Locator
	readonly formEmailInput: Locator
	readonly formPasswordInput: Locator
	readonly formAllowSsoSwitch: Locator
	readonly formStatusSwitch: Locator
	readonly formClusterContainer: Locator
	readonly formClusterInput: Locator
	readonly formSaveButton: Locator
	readonly formCancelButton: Locator

	// Modal Detail
	readonly modalDetail: Locator
	readonly modalDetailCloseButton: Locator

	// Modal Delete / Status Confirmation
	readonly modalDelete: Locator
	readonly modalDeleteTitle: Locator
	readonly optionDeactivateOrActivate: Locator
	readonly optionDeletePermanently: Locator
	readonly modalConfirmActionButton: Locator
	readonly modalCancelActionButton: Locator

	constructor(page: Page) {
		super(page)

		this.heading = page.getByRole('heading', { name: /^user management$/i })
		this.refreshButton = page.getByRole('button', { name: /^refresh$/i })

		// Summary Cards
		const statsGrid = page.locator('.grid.grid-cols-1, .grid.md\\:grid-cols-3').first()
		this.cardAllowSso = statsGrid.locator('> div').filter({ hasText: /total allow sso users/i })
		this.cardAllowSsoCount = this.cardAllowSso.locator('span.text-2xl')
		this.cardDisabled = statsGrid.locator('> div').filter({ hasText: /total disabled users/i })
		this.cardDisabledCount = this.cardDisabled.locator('span.text-2xl')
		this.cardClusterModule = statsGrid
			.locator('> div')
			.filter({ hasText: /users per cluster\/module/i })

		// Toolbar
		this.searchInput = page.getByPlaceholder(/search by username/i)
		this.createUserButton = page.getByRole('button', { name: /create user/i })

		// Table
		this.table = page.locator('table')
		this.tableRows = page.locator('table tbody tr')

		// Modal User Form
		this.modalForm = page.locator('[role="dialog"]').filter({
			has: page.getByRole('heading', { name: /create user|edit user/i })
		})
		this.formTitle = this.modalForm.getByRole('heading', { name: /create user|edit user/i })
		this.formFullNameInput = this.modalForm.getByPlaceholder(/enter full name/i)
		this.formUsernameInput = this.modalForm.getByPlaceholder(/enter username/i)
		this.formEmailInput = this.modalForm.getByPlaceholder(/user@patrolli\.local/i)
		this.formPasswordInput = this.modalForm.getByPlaceholder(/enter password/i)
		this.formAllowSsoSwitch = this.modalForm
			.locator('div')
			.filter({ has: page.locator('label', { hasText: /^allow sso/i }) })
			.last()
			.getByRole('switch')
		this.formStatusSwitch = this.modalForm
			.locator('div')
			.filter({ has: page.locator('label', { hasText: /^status/i }) })
			.last()
			.getByRole('switch')
		this.formClusterContainer = this.modalForm
			.locator('div')
			.filter({ has: page.locator('label', { hasText: /^cluster/i }) })
			.last()
		this.formClusterInput = this.modalForm.locator('.app-input-cluster [tabindex="0"]')
		this.formSaveButton = this.modalForm.getByRole('button', { name: /^save$/i })
		this.formCancelButton = this.modalForm.getByRole('button', { name: /^cancel$/i })

		// Modal Detail
		this.modalDetail = page.locator('[role="dialog"]').filter({
			has: page.getByRole('heading', { name: /read one user/i })
		})
		this.modalDetailCloseButton = this.modalDetail
			.locator('footer, .border-t')
			.getByRole('button', { name: /^close$/i })

		// Modal Delete / Confirm Action
		this.modalDelete = page.locator('[role="dialog"]').filter({
			has: page.getByRole('heading', { name: /confirm action/i })
		})
		this.modalDeleteTitle = this.modalDelete.getByRole('heading', { name: /confirm action/i })
		this.optionDeactivateOrActivate = this.modalDelete
			.locator('div')
			.filter({ hasText: /deactivate account|activate account/i })
			.first()
		this.optionDeletePermanently = this.modalDelete
			.locator('div')
			.filter({ hasText: /delete permanently/i })
			.first()
		this.modalConfirmActionButton = this.modalDelete
			.locator('button')
			.filter({ hasText: /deactivate user|activate user|delete user/i })
		this.modalCancelActionButton = this.modalDelete.getByRole('button', { name: /^cancel$/i })
	}

	async gotoUserManagement(): Promise<void> {
		await this.goto('/user-management')
		await this.page.waitForLoadState('domcontentloaded')
		await this.heading.waitFor({ state: 'visible', timeout: 15000 })
	}

	async navigateFromNavbar(): Promise<void> {
		const isMobile = (this.page.viewportSize()?.width ?? 1024) < 768
		if (isMobile) {
			const mobileMenuButton = this.page.locator('nav .md\\:hidden button').first()
			await mobileMenuButton.click()
			const drawerUserLink = this.page
				.locator('.md\\:hidden')
				.getByRole('link', { name: /^user management$/i })
			await drawerUserLink.waitFor({ state: 'visible', timeout: 5000 })
			await drawerUserLink.click()
		} else {
			const desktopUserLink = this.page
				.locator('nav')
				.getByRole('link', { name: /^user management$/i })
			await desktopUserLink.click()
		}
		await this.page.waitForURL(/\/user-management/, { timeout: 10000 })
		await this.heading.waitFor({ state: 'visible', timeout: 15000 })
	}

	async clickRefresh(): Promise<void> {
		const refreshPromise = this.page.waitForResponse(
			(res) => res.url().includes('/api/users') && res.status() < 400,
			{ timeout: 10000 }
		)
		await this.refreshButton.click()
		await refreshPromise
	}

	async searchUser(username: string): Promise<void> {
		await this.searchInput.fill(username)
		// Wait for debounced search request to finish
		await this.page.waitForResponse(
			(res) => res.url().includes('/api/users') && res.status() < 400,
			{ timeout: 10000 }
		)
	}

	getUserRow(username: string): Locator {
		return this.tableRows.filter({ hasText: `@${username}` }).first()
	}

	async openUserAction(
		username: string,
		actionName: 'View' | 'Edit' | 'Deactivate Account' | 'Activate Account' | 'Delete'
	): Promise<void> {
		const row = this.getUserRow(username)
		await row.waitFor({ state: 'visible', timeout: 10000 })
		const actionTrigger = row.locator('button').last()
		await actionTrigger.click()

		const dropdownMenu = this.page.locator('[role="menu"]')
		await dropdownMenu.waitFor({ state: 'visible', timeout: 5000 })
		const menuItem = dropdownMenu.getByRole('menuitem', {
			name: new RegExp(`^${actionName}$`, 'i')
		})
		await menuItem.click()
	}

	async fillCreateUserForm(data: {
		fullName: string
		username: string
		email?: string
		password: string
		allowSso?: boolean
	}): Promise<void> {
		await this.formFullNameInput.fill(data.fullName)
		await this.formUsernameInput.fill(data.username)
		if (data.email) {
			await this.formEmailInput.fill(data.email)
		}
		await this.formPasswordInput.fill(data.password)

		if (data.allowSso !== undefined) {
			const isChecked =
				(await this.formAllowSsoSwitch.getAttribute('aria-checked')) === 'true'
			if (isChecked !== data.allowSso) {
				await this.formAllowSsoSwitch.click()
			}
		}
	}
}

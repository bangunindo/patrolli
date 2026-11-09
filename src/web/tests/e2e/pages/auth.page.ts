// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { Locator, Page } from '@playwright/test'
import { BasePage } from './base.page'

export class AuthPage extends BasePage {
	readonly loginForm: Locator
	readonly usernameInput: Locator
	readonly passwordInput: Locator
	readonly submitButton: Locator
	readonly keycloakButton: Locator
	readonly errorMessage: Locator

	constructor(page: Page) {
		super(page)
		this.loginForm = page.locator('#kr-login-form')
		this.usernameInput = page.locator('input[name="b-login-form__email"]')
		this.passwordInput = page.locator('input[name="b-login-form__password"]')
		this.submitButton = page.locator('#kr-login-form__submit-btn')
		this.keycloakButton = page.locator('#kr-login-form__keycloak-btn')
		this.errorMessage = page.locator('.text-rose-500')
	}

	async gotoLogin() {
		await this.goto('/account/login')
		await this.loginForm.waitFor({ state: 'visible' })
	}

	async fillCredentials(username: string, password: string) {
		await this.usernameInput.fill(username)
		await this.passwordInput.fill(password)
	}

	async submit() {
		await this.submitButton.click()
	}

	async login(username: string, password: string) {
		await this.gotoLogin()
		await this.fillCredentials(username, password)
		await this.submit()
	}

	async clickKeycloakSSO() {
		await this.keycloakButton.click()
	}

	async loginKeycloak(username: string, password: string) {
		await this.gotoLogin()
		await this.clickKeycloakSSO()
		await this.page.waitForSelector('#username', { state: 'visible', timeout: 15_000 })
		await this.page.locator('#username').fill(username)
		await this.page.locator('#password').fill(password)
		await this.page.locator('#kc-login').click()
		await this.page.waitForURL((url) => !url.pathname.includes('/realms/'), {
			timeout: 20_000
		})
	}
}

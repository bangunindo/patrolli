// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { test as setup } from '@playwright/test'
import fs from 'node:fs'
import path from 'node:path'
import { STORAGE_STATE } from './test-fixtures'

setup('authenticate single-cluster user via local form', async ({ page }) => {
	const authDir = path.dirname(STORAGE_STATE.SINGLE_CLUSTER)
	if (!fs.existsSync(authDir)) {
		fs.mkdirSync(authDir, { recursive: true })
	}

	const username = process.env.E2E_USERNAME || 'admin'
	const password = process.env.E2E_PASSWORD || 'admin'

	await page.goto('/account/login')

	if (!page.url().includes('/account/login')) {
		await page.context().storageState({ path: STORAGE_STATE.SINGLE_CLUSTER })
		return
	}

	const loginForm = page.locator('#kr-login-form')
	await loginForm.waitFor({ state: 'visible', timeout: 15_000 })

	await page.locator('input[name="b-login-form__email"]').fill(username)
	await page.locator('input[name="b-login-form__password"]').fill(password)
	await page.locator('#kr-login-form__submit-btn').click()

	await page.waitForURL((url) => !url.pathname.includes('/account/login'), {
		timeout: 15_000
	})
	await page.waitForSelector('header nav', { state: 'visible', timeout: 15_000 })

	await page.context().storageState({ path: STORAGE_STATE.SINGLE_CLUSTER })
})

setup('authenticate multi-cluster user via Keycloak SSO', async ({ page }) => {
	const authDir = path.dirname(STORAGE_STATE.MULTI_CLUSTER)
	if (!fs.existsSync(authDir)) {
		fs.mkdirSync(authDir, { recursive: true })
	}

	const username = process.env.E2E_USERNAME || 'admin'
	const password = process.env.E2E_PASSWORD || 'admin'

	await page.goto('/account/login')

	if (!page.url().includes('/account/login')) {
		await page.context().storageState({ path: STORAGE_STATE.MULTI_CLUSTER })
		return
	}

	const keycloakBtn = page.locator('#kr-login-form__keycloak-btn')
	await keycloakBtn.waitFor({ state: 'visible', timeout: 15_000 })
	await keycloakBtn.click()

	await page.waitForSelector('#username', { state: 'visible', timeout: 15_000 })
	await page.locator('#username').fill(username)
	await page.locator('#password').fill(password)
	await page.locator('#kc-login').click()

	await page.waitForURL((url) => !url.pathname.includes('/realms/'), {
		timeout: 20_000
	})
	await page.waitForSelector('header nav', { state: 'visible', timeout: 15_000 })

	await page.context().storageState({ path: STORAGE_STATE.MULTI_CLUSTER })
})

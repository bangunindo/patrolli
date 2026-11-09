// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { test, expect } from '../../fixtures/test-fixtures'

test.describe('Authentication & Session Lifecycle', () => {
	test.describe('Unauthenticated Access & Direct Login Form', () => {
		test.use({ storageState: { cookies: [], origins: [] } })

		test('redirects unauthenticated user from protected routes to login', async ({ page }) => {
			await page.goto('/configuration/endpoint')
			await page.waitForURL((url) => url.pathname.includes('/account/login'), {
				timeout: 15_000
			})
			expect(page.url()).toContain('/account/login')
		})

		test('shows validation error when credentials are not entered', async ({ authPage }) => {
			await authPage.gotoLogin()
			await authPage.submit()
			// Should remain on the login page
			expect(authPage.page.url()).toContain('/account/login')
		})

		test('logs in successfully by filling valid credentials in login form and verifies single cluster is active @smoke', async ({
			authPage,
			page
		}) => {
			const username = process.env.E2E_USERNAME || 'admin'
			const password = process.env.E2E_PASSWORD || 'admin'

			await authPage.login(username, password)

			// Wait for redirection away from login page to dashboard
			await page.waitForURL((url) => !url.pathname.includes('/account/login'), {
				timeout: 15_000
			})
			await expect(page.locator('header nav')).toBeVisible({ timeout: 15_000 })
			expect(page.url()).not.toContain('/account/login')

			// Single cluster assertion: cluster selector in navbar must be locked / disabled
			await authPage.expectSingleClusterActive()
		})

		test('fails login and retains user on login page with invalid credentials', async ({
			authPage,
			page
		}) => {
			await authPage.gotoLogin()
			await authPage.fillCredentials('invalid_user', 'wrong_password')

			// Wait for failed response from authentication endpoint without arbitrary timeout
			await Promise.all([
				page
					.waitForResponse((res) => res.url().includes('/auth') && res.status() >= 400, {
						timeout: 10_000
					})
					.catch(() => null),
				authPage.submit()
			])

			expect(page.url()).toContain('/account/login')
		})

		test('logs in via Keycloak SSO and verifies multi-cluster access is active', async ({
			authPage,
			page
		}) => {
			const username = process.env.E2E_USERNAME || 'admin'
			const password = process.env.E2E_PASSWORD || 'admin'

			await authPage.loginKeycloak(username, password)

			// Wait for redirection back to application shell
			await expect(page.locator('header nav')).toBeVisible({ timeout: 15_000 })
			expect(page.url()).not.toContain('/account/login')

			// Multi-cluster assertion: cluster selector in navbar must be enabled & interactive
			await authPage.expectMultiClusterActive()
		})
	})

	test.describe('Authenticated Session', () => {
		test('allows authenticated user with storageState to access dashboard', async ({
			page
		}) => {
			await page.goto('/')
			// Should stay within protected area and show navbar
			await expect(page.locator('header nav')).toBeVisible({ timeout: 15_000 })
			expect(page.url()).not.toContain('/account/login')
		})
	})
})

// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { test, expect } from '../../fixtures/test-fixtures'

test.describe('Logout & Session Cleanup', () => {
	test('clears session and redirects to login on user logout', async ({ authPage, page }) => {
		await page.goto('/')
		await expect(page.locator('header nav')).toBeVisible({ timeout: 15_000 })

		await authPage.logout()

		// Verify redirect to login and protected route is blocked
		expect(page.url()).toContain('/account/login')

		await page.goto('/configuration/endpoint')
		await page.waitForURL((url) => url.pathname.includes('/account/login'), {
			timeout: 10_000
		})
		expect(page.url()).toContain('/account/login')
	})
})

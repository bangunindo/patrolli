// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { test, expect, STORAGE_STATE } from '../../fixtures/test-fixtures'

test.describe('Cluster Switching & Access Restrictions', () => {
	test.describe('Multi-Cluster Session (Keycloak SSO)', () => {
		test('allows switching active cluster and verifies active cluster update', async ({
			authPage,
			page
		}) => {
			await page.goto('/')
			await expect(page.locator('header nav')).toBeVisible({ timeout: 15_000 })

			// Verify multi-cluster mode is active
			await authPage.expectMultiClusterActive()

			// Check available options and switch cluster if multiple exist
			const trigger = await authPage.getActiveClusterTrigger()
			await trigger.click()
			await authPage.clusterDropdownTeleport.waitFor({ state: 'visible' })

			const optionsCount = await authPage.clusterOptions.count()
			if (optionsCount > 1) {
				const targetOptionText =
					(await authPage.clusterOptions.nth(1).textContent())?.trim() || ''
				await authPage.clusterOptions.nth(1).click()

				// Verify active cluster label updated
				await expect(trigger).toContainText(targetOptionText)
			} else {
				// Close dropdown if only 1 cluster configured
				await trigger.click()
			}
		})
	})

	test.describe('Single-Cluster Session (Local Form Login)', () => {
		test.use({ storageState: STORAGE_STATE.SINGLE_CLUSTER })

		test('restricts cluster switching and locks cluster selector in disabled state', async ({
			authPage,
			page
		}) => {
			await page.goto('/')
			await expect(page.locator('header nav')).toBeVisible({ timeout: 15_000 })

			// In single cluster session, selector must be locked & non-interactive
			await authPage.expectSingleClusterActive()
		})
	})
})

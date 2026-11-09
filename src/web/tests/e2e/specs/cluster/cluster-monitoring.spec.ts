// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { test, expect, STORAGE_STATE } from '../../fixtures/test-fixtures'

test.describe('Cluster Monitoring - Nodes Overview', () => {
	test.describe('Multi-Cluster Session (Keycloak SSO)', () => {
		test('renders cluster monitoring page with header, description, refresh button, and nodes table @smoke', async ({
			clusterPage
		}) => {
			await clusterPage.gotoCluster()

			await expect(clusterPage.pageHeader).toBeVisible()
			await expect(clusterPage.pageDescription).toBeVisible()
			await expect(clusterPage.refreshButton).toBeVisible()
			await expect(clusterPage.table).toBeVisible()

			// Verify required table headers
			await expect(clusterPage.tableHeaders).toContainText([
				'Node Name',
				'Host Count',
				'Status',
				'Leader',
				'Address',
				'Last Heartbeat'
			])

			// Verify at least one node is rendered in the cluster
			await expect(clusterPage.tableRows.first()).toBeVisible({ timeout: 10_000 })
		})

		test('triggers node re-fetch on refresh button click', async ({ clusterPage }) => {
			await clusterPage.gotoCluster()
			await expect(clusterPage.tableRows.first()).toBeVisible({ timeout: 10_000 })

			// Click refresh and wait for API response
			await clusterPage.clickRefresh()
			await expect(clusterPage.tableRows.first()).toBeVisible({ timeout: 10_000 })
		})

		test('navigates to configuration hosts filtered by node when clicking host count link', async ({
			clusterPage,
			page
		}) => {
			await clusterPage.gotoCluster()
			await expect(clusterPage.tableRows.first()).toBeVisible({ timeout: 10_000 })

			const firstRow = clusterPage.tableRows.first()
			const hostButton = firstRow.getByRole('button', { name: /host/i })
			await expect(hostButton).toBeVisible()

			await hostButton.click()
			await page.waitForURL((url) => url.pathname.includes('/configuration/host'), {
				timeout: 10_000
			})

			expect(page.url()).toContain('/configuration/host?monitored_by=')
			await expect(page.getByRole('heading', { name: /^host$/i })).toBeVisible({
				timeout: 10_000
			})
			await expect(page.locator('table')).toBeVisible()
		})

		test('navigates to cluster monitoring from navbar menu', async ({ clusterPage, page }) => {
			await page.goto('/configuration/host')
			await page.waitForLoadState('domcontentloaded')

			await clusterPage.navigateFromNavbar()
			await expect(clusterPage.pageHeader).toBeVisible()
		})
	})

	test.describe('Single-Cluster Session (Local Login)', () => {
		test.use({ storageState: STORAGE_STATE.SINGLE_CLUSTER })

		test('renders cluster monitoring table in single-cluster session', async ({
			clusterPage
		}) => {
			await clusterPage.gotoCluster()

			await expect(clusterPage.pageHeader).toBeVisible()
			await expect(clusterPage.table).toBeVisible()
			await expect(clusterPage.tableRows.first()).toBeVisible({ timeout: 10_000 })
		})
	})
})

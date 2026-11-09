// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { defineConfig, devices } from '@playwright/test'

const baseURL = process.env.E2E_BASE_URL || 'https://localhost:3000'

export default defineConfig({
	testDir: './tests/e2e',
	fullyParallel: true,
	forbidOnly: !!process.env.CI,
	retries: process.env.CI ? 2 : 0,
	workers: process.env.CI ? 1 : 2,
	reporter: [['github']],
	timeout: 60_000,
	expect: {
		timeout: 10_000
	},

	use: {
		baseURL,
		ignoreHTTPSErrors: true,
		trace: 'retain-on-failure',
		screenshot: 'only-on-failure',
		video: 'off'
	},

	projects: [
		{
			name: 'setup',
			testMatch: /.*\.setup\.ts/
		},
		{
			name: 'chromium',
			use: {
				...devices['Desktop Chrome'],
				storageState: 'tests/e2e/.auth/multi-cluster.json'
			},
			dependencies: ['setup']
		},
		{
			name: 'firefox',
			use: {
				...devices['Desktop Firefox'],
				storageState: 'tests/e2e/.auth/multi-cluster.json'
			},
			dependencies: ['setup']
		},
		{
			name: 'webkit',
			use: {
				...devices['Desktop Safari'],
				storageState: 'tests/e2e/.auth/multi-cluster.json'
			},
			dependencies: ['setup']
		},
		{
			name: 'mobile-chrome',
			use: {
				...devices['Pixel 7'],
				storageState: 'tests/e2e/.auth/multi-cluster.json'
			},
			dependencies: ['setup']
		}
	],

	webServer: {
		command: 'npm run build && npm run preview',
		url: baseURL,
		ignoreHTTPSErrors: true,
		reuseExistingServer: !process.env.CI,
		timeout: 180_000
	}
})

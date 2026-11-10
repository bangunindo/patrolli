// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { test as base, expect } from '@playwright/test'
import { AuthPage } from '../pages/auth.page'
import { ConfigurationPage } from '../pages/configuration.page'
import { PipelineBuilderPage } from '../pages/pipeline-builder.page'
import { PipelinePage } from '../pages/pipeline.page'
import { HostPage } from '../pages/host.page'
import { EndpointPage } from '../pages/endpoint.page'
import { ListenerPage } from '../pages/listener.page'
import { TemplatePage } from '../pages/template.page'
import { TemplateItemsPage } from '../pages/template-items.page'
import { ItemsPage } from '../pages/items.page'
import { SinkPage } from '../pages/sinks.page'
import { ClusterPage } from '../pages/cluster.page'
import { UserManagementPage } from '../pages/user-management.page'
import { E2eApiHelper } from './api.helper'

export const STORAGE_STATE = {
	SINGLE_CLUSTER: 'tests/e2e/.auth/single-cluster.json',
	MULTI_CLUSTER: 'tests/e2e/.auth/multi-cluster.json'
} as const

export interface ExtendedFixtures {
	authPage: AuthPage
	configurationPage: ConfigurationPage
	pipelinePage: PipelinePage
	pipelineBuilderPage: PipelineBuilderPage
	hostPage: HostPage
	endpointPage: EndpointPage
	listenerPage: ListenerPage
	templatePage: TemplatePage
	templateItemsPage: TemplateItemsPage
	itemsPage: ItemsPage
	sinkPage: SinkPage
	clusterPage: ClusterPage
	userManagementPage: UserManagementPage
	apiHelper: E2eApiHelper
	blockExternalFonts: void
	bffBoundaryGuard: void
}

export const test = base.extend<ExtendedFixtures>({
	authPage: async ({ page }, use) => {
		await use(new AuthPage(page))
	},

	configurationPage: async ({ page }, use) => {
		await use(new ConfigurationPage(page))
	},

	pipelinePage: async ({ page }, use) => {
		await use(new PipelinePage(page))
	},

	pipelineBuilderPage: async ({ page }, use) => {
		await use(new PipelineBuilderPage(page))
	},

	hostPage: async ({ page }, use) => {
		await use(new HostPage(page))
	},

	endpointPage: async ({ page }, use) => {
		await use(new EndpointPage(page))
	},

	listenerPage: async ({ page }, use) => {
		await use(new ListenerPage(page))
	},

	templatePage: async ({ page }, use) => {
		await use(new TemplatePage(page))
	},

	templateItemsPage: async ({ page }, use) => {
		await use(new TemplateItemsPage(page))
	},

	itemsPage: async ({ page }, use) => {
		await use(new ItemsPage(page))
	},

	sinkPage: async ({ page }, use) => {
		await use(new SinkPage(page))
	},

	clusterPage: async ({ page }, use) => {
		await use(new ClusterPage(page))
	},

	userManagementPage: async ({ page }, use) => {
		await use(new UserManagementPage(page))
	},

	apiHelper: async ({ request }, use) => {
		await use(new E2eApiHelper(request))
	},

	// Abort external fonts network round-trips to speed up page loads
	blockExternalFonts: [
		async ({ page }, use) => {
			await page.route(/fonts\.(googleapis|gstatic)\.com/, (route) => route.abort())
			await use()
		},
		{ auto: true }
	],

	// Invariant: verify browser traffic stays within Nitro BFF & allowed origins
	bffBoundaryGuard: [
		async ({ page, baseURL }, use) => {
			const allowedOrigin = new URL(baseURL || 'https://localhost:3000').origin
			const externalViolations: string[] = []

			page.on('request', (request) => {
				const resourceType = request.resourceType()
				if (resourceType === 'fetch' || resourceType === 'xhr') {
					try {
						const reqUrl = new URL(request.url())
						const isLocalOrigin = reqUrl.origin === allowedOrigin
						const isKeycloakAuth = reqUrl.origin.includes('kc.localtest.me')
						const isStaticCdn =
							reqUrl.origin.includes('api.iconify.design') ||
							reqUrl.origin.includes('fonts.googleapis.com') ||
							reqUrl.origin.includes('fonts.gstatic.com')
						const isAllowedPath =
							reqUrl.pathname.startsWith('/api/') ||
							reqUrl.pathname.startsWith('/auth/') ||
							reqUrl.pathname.startsWith('/_nuxt/')

						if (!isLocalOrigin && !isKeycloakAuth && !isStaticCdn) {
							externalViolations.push(`[${request.method()}] ${request.url()}`)
						} else if (isLocalOrigin && !isAllowedPath) {
							// Allow same-origin page loads and assets
						}
					} catch {
						// Invalid URL, skip check
					}
				}
			})

			await use()

			expect(
				externalViolations,
				`Direct external requests found bypassing BFF:\n${externalViolations.join('\n')}`
			).toEqual([])
		},
		{ auto: true }
	]
})

export { expect }

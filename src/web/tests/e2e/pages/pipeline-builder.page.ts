// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { Locator, Page } from '@playwright/test'
import { BasePage } from './base.page'

export class PipelineBuilderPage extends BasePage {
	readonly canvas: Locator
	readonly nodes: Locator
	readonly sourceNode: Locator
	readonly processorNodes: Locator
	readonly sinkNodes: Locator
	readonly edges: Locator

	// Navigation & Controls
	readonly backButton: Locator
	readonly pipelineTitle: Locator
	readonly statusBadge: Locator
	readonly saveButton: Locator
	readonly autoLayoutButton: Locator

	// Node Toolbar
	readonly emptyStateCreateButton: Locator
	readonly toolbarAddSource: Locator
	readonly toolbarAddProcessor: Locator
	readonly toolbarAddSink: Locator

	// Processor Group Drawer
	readonly processorDrawer: Locator
	readonly buttonAddProcessorInDrawer: Locator
	readonly buttonCloseDrawer: Locator

	// Unsaved Changes Modal
	readonly modalUnsaved: Locator
	readonly buttonCancelLeave: Locator
	readonly buttonDiscardLeave: Locator
	readonly buttonSaveAndLeave: Locator

	constructor(page: Page) {
		super(page)

		this.canvas = page.locator('.vue-flow')
		this.nodes = page.locator('.vue-flow__node')
		this.sourceNode = page
			.locator('.vue-flow__node-source, .vue-flow__node')
			.filter({ hasText: /source/i })
			.first()
		this.processorNodes = page
			.locator('.vue-flow__node-processor, .vue-flow__node')
			.filter({ hasText: /processor/i })
		this.sinkNodes = page
			.locator('.vue-flow__node-sink, .vue-flow__node')
			.filter({ hasText: /(sink target|sink)/i })
		this.edges = page.locator('.vue-flow__edge')

		// Navigation & Top controls
		this.backButton = page.getByRole('button', { name: /^back$/i })
		this.pipelineTitle = page.locator('h1')
		this.statusBadge = page
			.locator('h1 ~ span, span:has-text("Running"), span:has-text("Draft")')
			.first()
		this.saveButton = page.getByRole('button', { name: /(save changes|saved)/i })
		this.autoLayoutButton = page.locator('button[title="Auto Layout"]').first()

		// Toolbar controls
		this.emptyStateCreateButton = page
			.locator('.canvas-empty-state')
			.getByRole('button', { name: /^create$/i })
		this.toolbarAddSource = page
			.locator('button[title="Click to add or drag onto canvas"]')
			.filter({ hasText: 'Source' })
			.first()
		this.toolbarAddProcessor = page
			.locator('button[title="Click to add or drag onto canvas"]')
			.filter({ hasText: 'Processor Group' })
			.first()
		this.toolbarAddSink = page
			.locator('button[title="Click to add or drag onto canvas"]')
			.filter({ hasText: 'Sink' })
			.first()

		// Processor Group Drawer
		this.processorDrawer = page
			.locator('[role="dialog"], aside, .drawer')
			.filter({ hasText: /processor/i })
			.first()
		this.buttonAddProcessorInDrawer = this.processorDrawer.getByRole('button', {
			name: /add processor/i
		})
		this.buttonCloseDrawer = this.processorDrawer
			.locator('button')
			.filter({ hasText: /✕|close/i })
			.first()

		// Unsaved Changes Modal
		this.modalUnsaved = page.locator('[role="dialog"]').filter({
			has: page.getByRole('heading', { name: /unsaved changes/i })
		})
		this.buttonCancelLeave = this.modalUnsaved.getByRole('button', { name: /^cancel$/i })
		this.buttonDiscardLeave = this.modalUnsaved.getByRole('button', {
			name: /discard & leave/i
		})
		this.buttonSaveAndLeave = this.modalUnsaved.getByRole('button', {
			name: /save & leave/i
		})
	}

	async gotoPipelineList(): Promise<void> {
		await this.goto('/pipeline')
		await this.page.waitForSelector('h1, h2, table, [role="table"]', { state: 'visible' })
	}

	async openPipelineBuilderByName(name: string): Promise<void> {
		await this.gotoPipelineList()
		const row = this.page.locator('table tbody tr').filter({ hasText: name }).first()
		const link = row.locator('div.cursor-pointer, a').first()
		await link.click()
		await this.page.waitForURL(/\/pipeline\/.*\/builder\/.*/)
		await this.page.waitForLoadState('domcontentloaded')
	}

	async addSourceNode(): Promise<void> {
		if (await this.emptyStateCreateButton.isVisible()) {
			await this.emptyStateCreateButton.click()
		} else if (await this.toolbarAddSource.isVisible()) {
			await this.toolbarAddSource.click()
		}
	}

	async addProcessorGroupNode(): Promise<void> {
		await this.toolbarAddProcessor.waitFor({ state: 'visible', timeout: 5_000 })
		await this.toolbarAddProcessor.click()
	}

	async addSinkNode(): Promise<void> {
		await this.toolbarAddSink.waitFor({ state: 'visible', timeout: 5_000 })
		await this.toolbarAddSink.click()
	}

	async triggerAutoLayout(): Promise<void> {
		await this.autoLayoutButton.click({ force: true })
	}

	async clickSave(): Promise<void> {
		await this.saveButton.scrollIntoViewIfNeeded()
		await this.saveButton.click({ force: true })
	}
}

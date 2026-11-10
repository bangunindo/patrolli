// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { APIRequestContext } from '@playwright/test'

export class E2eApiHelper {
	constructor(private readonly request: APIRequestContext) {}

	async deleteTemplateByName(name: string): Promise<void> {
		try {
			const res = await this.request.get('/api/templates', { params: { name } })
			if (res.ok()) {
				const json = await res.json()
				const templates = json.data?.templates || []
				for (const t of templates) {
					if (t.name === name || t.name.startsWith(name)) {
						await this.request.delete(`/api/templates/${t.template_id}`)
					}
				}
			}
		} catch {
			// Ignore teardown cleanup errors
		}
	}

	async deleteHostByName(name: string): Promise<void> {
		try {
			const res = await this.request.get('/api/hosts', { params: { name } })
			if (res.ok()) {
				const json = await res.json()
				const hosts = json.data?.hosts || []
				for (const h of hosts) {
					if (h.name === name || h.name.startsWith(name)) {
						await this.request.delete(`/api/hosts/${h.host_id}`)
					}
				}
			}
		} catch {
			// Ignore teardown cleanup errors
		}
	}

	async deleteEndpointByName(name: string): Promise<void> {
		try {
			const res = await this.request.get('/api/endpoints', { params: { name } })
			if (res.ok()) {
				const json = await res.json()
				const endpoints = json.data?.endpoints || []
				for (const ep of endpoints) {
					if (ep.name === name || ep.name.startsWith(name)) {
						await this.request.delete(`/api/endpoints/${ep.endpoint_id}`)
					}
				}
			}
		} catch {
			// Ignore teardown cleanup errors
		}
	}

	async deleteListenerByName(name: string): Promise<void> {
		try {
			const res = await this.request.get('/api/listeners', { params: { name } })
			if (res.ok()) {
				const json = await res.json()
				const listeners = json.data?.listeners || []
				for (const l of listeners) {
					if (l.name === name || l.name.startsWith(name)) {
						await this.request.delete(`/api/listeners/${l.listener_id}`)
					}
				}
			}
		} catch {
			// Ignore teardown cleanup errors
		}
	}

	async deleteSinkByName(name: string): Promise<void> {
		try {
			const res = await this.request.get('/api/sinks', { params: { name } })
			if (res.ok()) {
				const json = await res.json()
				const sinks = json.data?.sinks || []
				for (const s of sinks) {
					if (s.name === name || s.name.startsWith(name)) {
						await this.request.delete(`/api/sinks/${s.sink_id}`)
					}
				}
			}
		} catch {
			// Ignore teardown cleanup errors
		}
	}

	async deleteItemByName(name: string): Promise<void> {
		try {
			const res = await this.request.get('/api/items', { params: { name } })
			if (res.ok()) {
				const json = await res.json()
				const items = json.data?.items || []
				for (const i of items) {
					if (i.name === name || i.name.startsWith(name)) {
						await this.request.delete(`/api/items/${i.item_id}`)
					}
				}
			}
		} catch {
			// Ignore teardown cleanup errors
		}
	}
}

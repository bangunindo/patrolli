// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

export default {
	nodes: [
		{
			id: 'host-1',
			type: 'host',
			data: {
				host_template_id: '019f0333-f653-7f41-983d-6969f94d7a9f',
				host_id: '019f0333-f645-7972-b3ec-12f031f9fd7e',
				name: 'test',
				adm_status: 1,
				labels: '{"region":"jakarta","cluster":"gcloud-1"}',
				last_updated: '2026-06-30T04:10:41.245626Z',
				monitored_by: '019d8eec-203e-7e83-895e-6a07d6f2f58e',
				templates: [],
				item_count: 0
			},
			position: { x: 65, y: 150 }
		},
		{
			id: 'endpoint-1',
			type: 'endpoint',
			position: { x: 65 * 4, y: 250 }
		},
		{
			id: 'item-1',
			type: 'item',
			position: { x: 65 * 7, y: 150 },
			data: {
				name: 'Item 1'
			}
		},
		{
			id: 'processor-button-1',
			type: 'processor-button',
			position: { x: 65 * 10, y: 150 }
		}
	],
	edges: [
		{
			id: `e-host-listener`,
			source: 'host-1',
			target: 'listener-1'
		},
		{
			id: `e-host-endpoint`,
			source: 'host-1',
			target: 'endpoint-1'
		},
		{
			id: `e-endpoint-item`,
			source: 'endpoint-1',
			target: 'item-1'
		},
		{
			id: `e-listener-item`,
			source: 'listener-1',
			target: 'item-1'
		},
		{
			id: `e-item-processor`,
			source: 'item-1',
			target: 'processor-button-1',
			type: 'item-processor'
		},
		{
			id: `e-processor-sink`,
			source: 'processor-1',
			target: 'sink-1'
		}
	]
}

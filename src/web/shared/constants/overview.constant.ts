// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import { Position } from '@vue-flow/core'

export default {
	nodes: [
		{
			id: 'host-1',
			type: 'host',
			data: {
				label: 'Root Node'
			},
			status: true,
			position: { x: 50, y: 150 },
			sourcePosition: Position.Right
		},
		{
			id: 'listener-1',
			type: 'listener',
			label: 'Root Node',
			status: true,
			position: { x: 50 * 4, y: 50 },
			sourcePosition: Position.Right
		},
		{
			id: 'endpoint-1',
			type: 'endpoint',
			label: 'Root Node',
			position: { x: 50 * 4, y: 250 },
			sourcePosition: Position.Right
		},
		{
			id: 'item-1',
			type: 'items',
			label: 'Root Node',
			position: { x: 50 * 7, y: 150 },
			sourcePosition: Position.Right
		},
		{
			id: 'processor-1',
			type: 'processor',
			label: 'Root Node',
			position: { x: 50 * 10, y: 150 },
			sourcePosition: Position.Right
		},
		{
			id: 'sink-1',
			type: 'sink',
			label: 'Root Node',
			position: { x: 50 * 13, y: 150 },
			sourcePosition: Position.Right
		}
	],
	edges: [
		{
			id: `e-host-listener`,
			type: 'smoothstep',
			source: 'host-1',
			target: 'listener-1',
			animated: false
		},
		{
			id: `e-host-endpoint`,
			type: 'smoothstep',
			source: 'host-1',
			target: 'endpoint-1',
			animated: true
		},
		{
			id: `e-endpoint-item`,
			type: 'smoothstep',
			source: 'endpoint-1',
			target: 'item-1',
			animated: true
		},
		{
			id: `e-listener-item`,
			type: 'smoothstep',
			source: 'listener-1',
			target: 'item-1',
			animated: false
		},
		{
			id: `e-item-processor`,
			type: 'smoothstep',
			source: 'item-1',
			target: 'processor-1',
			animated: true
		},
		{
			id: `e-processor-sink`,
			type: 'smoothstep',
			source: 'processor-1',
			target: 'sink-1',
			animated: true
		}
	]
}

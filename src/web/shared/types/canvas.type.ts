// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { Edge, Node } from '@vue-flow/core'

export enum ENodeType {
	'Node' = 'node',
	'Edge' = 'edge'
}

export type INode = Node & {
	node_id?: string
	select_type?: ENodeType
	nodeType?: string
}

export type IEdge = Edge & {
	select_type?: ENodeType
}

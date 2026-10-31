// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { Connection } from '@vue-flow/core'
import type { IBaseResponse } from '@@/shared/types/api.type'
import type { IEdge, INode } from '@@/shared/types/canvas.type'

export interface IPipelineBuilderListResponse extends IBaseResponse {
	data?: {
		nodes: INode[]
		edges: IEdge[]
	}
}

export interface ConnectionLine extends Partial<Connection> {
	sourceNode?: INode
	targetNode?: INode
}

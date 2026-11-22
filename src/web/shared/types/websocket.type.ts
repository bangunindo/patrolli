// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

import type { ICluster } from '@@/shared/types/cluster.type'
import type WebSocket from 'ws'

export interface INitroWsSession {
	upstream?: WebSocket
	targetCluster?: ICluster
	path?: string
	pendingMessages?: string[]
}
